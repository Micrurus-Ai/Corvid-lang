use super::{RagChunk, RagDocument, RagEmbeddingRecord, RagSearchHit};
use crate::errors::RuntimeError;
use crate::provenance::GroundedValue;
use crate::tracing::now_ms;
use rusqlite::{params, Connection};
use std::path::Path;

pub struct RagSqliteIndex {
    conn: Connection,
}

impl RagSqliteIndex {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let conn = Connection::open(path.as_ref()).map_err(|err| {
            RuntimeError::Other(format!(
                "failed to open RAG index `{}`: {err}",
                path.as_ref().display()
            ))
        })?;
        let index = Self { conn };
        index.init()?;
        Ok(index)
    }

    pub fn open_in_memory() -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory().map_err(|err| {
            RuntimeError::Other(format!("failed to open RAG memory index: {err}"))
        })?;
        let index = Self { conn };
        index.init()?;
        Ok(index)
    }

    pub fn insert_document(&self, document: &RagDocument) -> Result<(), RuntimeError> {
        Self::write_document(&self.conn, document)
    }

    fn write_document(conn: &Connection, document: &RagDocument) -> Result<(), RuntimeError> {
        conn.execute(
            "insert or replace into rag_documents (id, source, media_type, text)
                 values (?1, ?2, ?3, ?4)",
            params![
                document.id,
                document.source,
                document.media_type,
                document.text
            ],
        )
        .map_err(|err| RuntimeError::Other(format!("failed to insert RAG document: {err}")))?;
        Ok(())
    }

    pub fn insert_chunks(&mut self, chunks: &[RagChunk]) -> Result<(), RuntimeError> {
        let tx = self.conn.transaction().map_err(|err| {
            RuntimeError::Other(format!("failed to start RAG chunk insert: {err}"))
        })?;
        Self::write_chunks(&tx, chunks)?;
        tx.commit()
            .map_err(|err| RuntimeError::Other(format!("failed to commit RAG chunks: {err}")))?;
        Ok(())
    }

    fn write_chunks(conn: &Connection, chunks: &[RagChunk]) -> Result<(), RuntimeError> {
        for chunk in chunks {
            conn.execute(
                "insert or replace into rag_chunks
                 (chunk_id, doc_id, source, text, start_char, end_char, provenance_key)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    chunk.chunk_id,
                    chunk.doc_id,
                    chunk.source,
                    chunk.text,
                    chunk.start_char as i64,
                    chunk.end_char as i64,
                    chunk.provenance_key
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to insert RAG chunk: {err}")))?;
        }
        Ok(())
    }

    pub(crate) fn replace_document(
        &mut self,
        document: &RagDocument,
        chunks: &[RagChunk],
        embeddings: &[RagEmbeddingRecord],
    ) -> Result<(), RuntimeError> {
        let failure = |err| RuntimeError::Other(format!("failed to replace RAG document: {err}"));
        let tx = self.conn.transaction().map_err(failure)?;
        tx.execute(
            "delete from rag_chunk_embeddings where chunk_id in
            (select chunk_id from rag_chunks where doc_id = ?1)",
            [&document.id],
        )
        .map_err(failure)?;
        tx.execute("delete from rag_chunks where doc_id = ?1", [&document.id])
            .map_err(failure)?;
        Self::write_document(&tx, document)?;
        Self::write_chunks(&tx, chunks)?;
        Self::write_embeddings(&tx, embeddings)?;
        tx.commit().map_err(failure)
    }

    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<RagChunk>, RuntimeError> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like = format!("%{escaped}%");
        self.lexical_query(
            "select chunk_id, doc_id, source, text, start_char, end_char, provenance_key
             from rag_chunks where text like ?1 escape '\\' order by chunk_id limit ?2",
            &like,
            limit,
        )
    }

    pub(crate) fn search_terms(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RagChunk>, RuntimeError> {
        let terms: Vec<_> = query
            .split_whitespace()
            .filter(|term| term.chars().count() >= 2)
            .collect();
        let terms = serde_json::json!(terms).to_string();
        self.lexical_query(
            "select c.chunk_id, c.doc_id, c.source, c.text, c.start_char, c.end_char, c.provenance_key
             from rag_chunks c join json_each(?1) t on instr(lower(c.text), lower(t.value)) > 0
             group by c.chunk_id order by count(*) desc, c.chunk_id limit ?2",
            &terms, limit,
        )
    }

    fn lexical_query(
        &self,
        sql: &str,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<RagChunk>, RuntimeError> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|err| RuntimeError::Other(format!("failed to prepare RAG search: {err}")))?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], |row| {
                Ok(RagChunk {
                    chunk_id: row.get(0)?,
                    doc_id: row.get(1)?,
                    source: row.get(2)?,
                    text: row.get(3)?,
                    start_char: row.get::<_, i64>(4)? as usize,
                    end_char: row.get::<_, i64>(5)? as usize,
                    provenance_key: row.get(6)?,
                })
            })
            .map_err(|err| RuntimeError::Other(format!("failed to query RAG chunks: {err}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|err| RuntimeError::Other(format!("failed to read RAG chunk: {err}")))?,
            );
        }
        Ok(out)
    }

    pub fn search_grounded_text(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<GroundedValue<RagChunk>>, RuntimeError> {
        let source_name = format!("std.rag.search_text:{query}");
        Ok(self
            .search_text(query, limit)?
            .into_iter()
            .map(|chunk| GroundedValue::new(chunk, grounded_chunk_chain(&source_name)))
            .collect())
    }

    pub fn insert_embedding_vectors(
        &mut self,
        embeddings: &[RagEmbeddingRecord],
    ) -> Result<(), RuntimeError> {
        let tx = self.conn.transaction().map_err(|err| {
            RuntimeError::Other(format!("failed to start RAG embedding insert: {err}"))
        })?;
        Self::write_embeddings(&tx, embeddings)?;
        tx.commit().map_err(|err| {
            RuntimeError::Other(format!("failed to commit RAG embeddings: {err}"))
        })?;
        Ok(())
    }

    fn write_embeddings(
        conn: &Connection,
        embeddings: &[RagEmbeddingRecord],
    ) -> Result<(), RuntimeError> {
        for embedding in embeddings {
            let payload = serde_json::to_string(&embedding.values).map_err(|err| {
                RuntimeError::Other(format!("failed to encode RAG embedding vector: {err}"))
            })?;
            conn.execute(
                "insert or replace into rag_chunk_embeddings
                 (chunk_id, dimension, values_json)
                 values (?1, ?2, ?3)",
                params![embedding.chunk_id, embedding.values.len() as i64, payload],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to insert RAG embedding: {err}")))?;
        }
        Ok(())
    }

    pub fn search_embeddings(
        &self,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<RagSearchHit>, RuntimeError> {
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "select c.chunk_id, c.doc_id, c.source, c.text, c.start_char, c.end_char, c.provenance_key,
                        e.values_json
                 from rag_chunks c
                 join rag_chunk_embeddings e on e.chunk_id = c.chunk_id
                 where e.dimension = ?1",
            )
            .map_err(|err| RuntimeError::Other(format!("failed to prepare RAG embedding search: {err}")))?;
        let rows = stmt
            .query_map(params![query.len() as i64], |row| {
                Ok((
                    RagChunk {
                        chunk_id: row.get(0)?,
                        doc_id: row.get(1)?,
                        source: row.get(2)?,
                        text: row.get(3)?,
                        start_char: row.get::<_, i64>(4)? as usize,
                        end_char: row.get::<_, i64>(5)? as usize,
                        provenance_key: row.get(6)?,
                    },
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|err| RuntimeError::Other(format!("failed to query RAG embeddings: {err}")))?;
        let mut hits = Vec::new();
        for row in rows {
            let (chunk, values_json) = row.map_err(|err| {
                RuntimeError::Other(format!("failed to read RAG embedding row: {err}"))
            })?;
            let values: Vec<f32> = serde_json::from_str(&values_json).map_err(|err| {
                RuntimeError::Other(format!("failed to decode RAG embedding vector: {err}"))
            })?;
            let score = cosine_similarity(query, &values);
            hits.push(RagSearchHit { chunk, score });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.chunk.chunk_id.cmp(&right.chunk.chunk_id))
        });
        if hits.len() > limit {
            hits.truncate(limit);
        }
        Ok(hits)
    }

    pub fn search_grounded_embeddings(
        &self,
        query_label: &str,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<GroundedValue<RagChunk>>, RuntimeError> {
        let source_name = format!("std.rag.search_embeddings:{query_label}");
        Ok(self
            .search_embeddings(query, limit)?
            .into_iter()
            .map(|hit| GroundedValue::new(hit.chunk, grounded_chunk_chain(&source_name)))
            .collect())
    }

    fn init(&self) -> Result<(), RuntimeError> {
        self.conn
            .execute_batch(
                "create table if not exists rag_documents (
                    id text primary key,
                    source text not null,
                    media_type text not null,
                    text text not null
                );
                create table if not exists rag_chunks (
                    chunk_id text primary key,
                    doc_id text not null,
                    source text not null,
                    text text not null,
                    start_char integer not null,
                    end_char integer not null,
                    provenance_key text not null,
                    foreign key(doc_id) references rag_documents(id)
                );
                create index if not exists rag_chunks_doc_id on rag_chunks(doc_id);
                create index if not exists rag_chunks_provenance_key on rag_chunks(provenance_key);
                create table if not exists rag_chunk_embeddings (
                    chunk_id text primary key,
                    dimension integer not null,
                    values_json text not null,
                    foreign key(chunk_id) references rag_chunks(chunk_id)
                );
                create index if not exists rag_chunk_embeddings_dimension on rag_chunk_embeddings(dimension);",
            )
            .map_err(|err| RuntimeError::Other(format!("failed to initialize RAG index: {err}")))?;
        Ok(())
    }
}

fn grounded_chunk_chain(source_name: &str) -> crate::provenance::ProvenanceChain {
    crate::provenance::ProvenanceChain::with_retrieval(source_name, now_ms())
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        let (lhs, rhs) = (f64::from(*lhs), f64::from(*rhs));
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        (dot / (left_norm.sqrt() * right_norm.sqrt())) as f32
    }
}
