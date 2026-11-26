//! `corvid migrate` clap argument tree — slice 27 / migrations
//! surface, decomposed in Phase 20j-A1.
//!
//! Owns the [`MigrateCommand`] subcommand enum that the
//! `corvid migrate plan|status|up|down` dispatch arms in main.rs
//! consume.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Write the migration that brings the stored tables to the program's declarations.
    Plan {
        /// The Corvid source whose `table` declarations to plan.
        source: PathBuf,
        /// Directory containing ordered `.sql` migration files.
        #[arg(long, value_name = "DIR", default_value = "migrations")]
        dir: PathBuf,
        /// Print the planned migration without writing it.
        #[arg(long)]
        dry_run: bool,
        /// For a Postgres program: the environment variable holding a
        /// scratch database's connection string. The planner applies the
        /// migrations there inside a transaction it rolls back, and refuses
        /// a database that is not empty.
        #[arg(long, value_name = "ENV_VAR")]
        scratch: Option<String>,
    },
    /// Report applied, pending, and drifted migrations.
    Status {
        /// Directory containing ordered `.sql` migration files.
        #[arg(long, value_name = "DIR", default_value = "migrations")]
        dir: PathBuf,
        /// State file used to record applied migration checksums.
        #[arg(
            long,
            value_name = "PATH",
            default_value = "target/corvid-migrations.json"
        )]
        state: PathBuf,
        /// SQLite database file migrations are checked against.
        #[arg(long, value_name = "PATH", default_value = "target/corvid.sqlite")]
        database: PathBuf,
        /// Show what would be checked without writing state.
        #[arg(long)]
        dry_run: bool,
        /// The Corvid program whose `database` block names the database,
        /// engine, connection string and TLS policy, instead of `--database`.
        #[arg(long, value_name = "SOURCE", conflicts_with = "database")]
        program: Option<PathBuf>,
    },
    /// Apply pending migrations in order.
    Up {
        /// Directory containing ordered `.sql` migration files.
        #[arg(long, value_name = "DIR", default_value = "migrations")]
        dir: PathBuf,
        /// State file used to record applied migration checksums.
        #[arg(
            long,
            value_name = "PATH",
            default_value = "target/corvid-migrations.json"
        )]
        state: PathBuf,
        /// SQLite database file migrations are executed against.
        #[arg(long, value_name = "PATH", default_value = "target/corvid.sqlite")]
        database: PathBuf,
        /// Report pending migrations without applying them.
        #[arg(long)]
        dry_run: bool,
        /// The Corvid program whose `database` block names the database,
        /// engine, connection string and TLS policy, instead of `--database`.
        #[arg(long, value_name = "SOURCE", conflicts_with = "database")]
        program: Option<PathBuf>,
    },
    /// Roll back the latest migration when a down migration exists.
    Down {
        /// Directory containing ordered `.sql` migration files.
        #[arg(long, value_name = "DIR", default_value = "migrations")]
        dir: PathBuf,
        /// Directory containing reviewed rollback SQL files named `<migration>.down.sql`.
        #[arg(long, value_name = "DIR", default_value = "migrations/down")]
        down_dir: PathBuf,
        /// State file used to record applied migration checksums.
        #[arg(
            long,
            value_name = "PATH",
            default_value = "target/corvid-migrations.json"
        )]
        state: PathBuf,
        /// SQLite database file associated with migration state.
        #[arg(long, value_name = "PATH", default_value = "target/corvid.sqlite")]
        database: PathBuf,
        /// Report the rollback candidate without mutating state.
        #[arg(long)]
        dry_run: bool,
        /// The Corvid program whose `database` block names the database,
        /// engine, connection string and TLS policy, instead of `--database`.
        #[arg(long, value_name = "SOURCE", conflicts_with = "database")]
        program: Option<PathBuf>,
    },
}
