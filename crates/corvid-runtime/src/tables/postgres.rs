//! The Postgres connection behind tables declared `engine: postgres`
//! (B09 P4).
//!
//! The connection is asynchronous, so a table operation never blocks
//! the interpreter's runtime. Values are bound as parameters through the
//! same conversion `std.db` uses, and rows are read back through its
//! cell reader, so both database surfaces agree on what a value is.
//!
//! `tls: required` encrypts the connection and verifies the server's
//! certificate against the standard web roots; a server whose
//! certificate does not verify is refused. `tls: disabled` connects
//! without encryption, as the source declared.

use crate::db::{db_value_to_postgres_param, postgres_cell_value, DbValue};
use crate::errors::RuntimeError;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};

/// One open connection to a Postgres database.
pub struct PostgresTables {
    client: tokio_postgres::Client,
}

impl PostgresTables {
    pub async fn connect(url: &str, tls: bool) -> Result<Self, RuntimeError> {
        let client = if tls {
            let (client, connection) = tokio_postgres::connect(url, VerifiedTls::new()?)
                .await
                .map_err(|e| connect_error(e, tls))?;
            tokio::spawn(connection);
            client
        } else {
            let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
                .await
                .map_err(|e| connect_error(e, tls))?;
            tokio::spawn(connection);
            client
        };
        Ok(Self { client })
    }

    /// Whether the server or the network closed the connection.
    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Run a statement; the number of rows it changed.
    pub async fn execute(&self, sql: &str, params: &[DbValue]) -> Result<u64, RuntimeError> {
        let params = params.iter().map(db_value_to_postgres_param).collect::<Vec<_>>();
        let refs = params.iter().map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync)).collect::<Vec<_>>();
        self.client.execute(sql, &refs).await.map_err(statement_error)
    }

    /// Run a query; each row's values for `columns`, in order.
    pub async fn select(&self, sql: &str, params: &[DbValue], columns: &[String]) -> Result<Vec<Vec<DbValue>>, RuntimeError> {
        let params = params.iter().map(db_value_to_postgres_param).collect::<Vec<_>>();
        let refs = params.iter().map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync)).collect::<Vec<_>>();
        let rows = self.client.query(sql, &refs).await.map_err(statement_error)?;
        Ok(rows.iter().map(|row| columns.iter().map(|c| postgres_cell_value(row, c)).collect()).collect())
    }
}

fn connect_error(error: tokio_postgres::Error, tls: bool) -> RuntimeError {
    let hint = if tls {
        "; the `database` block says `tls: required`, so the server's certificate must verify against the standard web roots"
    } else {
        ""
    };
    RuntimeError::Other(format!("cannot connect to the Postgres database: {error}{hint}"))
}

/// A failed statement. The SQL is Corvid's own and every value is a
/// bound parameter, so the message names the cause, never the values.
fn statement_error(error: tokio_postgres::Error) -> RuntimeError {
    let detail = error.as_db_error().map(|db| db.message().to_string()).unwrap_or_else(|| error.to_string());
    let missing = error.as_db_error().is_some_and(|db| db.code() == &tokio_postgres::error::SqlState::UNDEFINED_TABLE);
    if missing {
        // `named` in the parent module turns this into the migration hint.
        return RuntimeError::Other(format!("no such table: {detail}"));
    }
    RuntimeError::Other(format!("Postgres error: {detail}; values redacted"))
}

/// TLS through rustls, verifying the server against the web roots.
/// `corvid migrate` connects through it too, so a `tls: required`
/// database is verified the same way by every command.
#[derive(Clone)]
pub struct VerifiedTls {
    config: Arc<rustls::ClientConfig>,
}

impl VerifiedTls {
    pub fn new() -> Result<Self, RuntimeError> {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| RuntimeError::Other(format!("cannot set up TLS for Postgres: {e}")))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self { config: Arc::new(config) })
    }
}

impl<S> MakeTlsConnect<S> for VerifiedTls
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = VerifiedStream<S>;
    type TlsConnect = VerifiedConnect;
    type Error = io::Error;

    fn make_tls_connect(&mut self, domain: &str) -> Result<VerifiedConnect, io::Error> {
        let name = rustls::pki_types::ServerName::try_from(domain.to_string())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        Ok(VerifiedConnect { config: self.config.clone(), name })
    }
}

pub struct VerifiedConnect {
    config: Arc<rustls::ClientConfig>,
    name: rustls::pki_types::ServerName<'static>,
}

impl<S> TlsConnect<S> for VerifiedConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = VerifiedStream<S>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<VerifiedStream<S>, io::Error>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move {
            let connector = tokio_rustls::TlsConnector::from(self.config);
            connector.connect(self.name, stream).await.map(VerifiedStream)
        })
    }
}

pub struct VerifiedStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for VerifiedStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for VerifiedStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> TlsStream for VerifiedStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}
