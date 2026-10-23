//! Database migration command applying versioned refinery migrations.

use anyhow::{Context, Result};
use clap::Args;
use dbsrt_index::DbClient;

use crate::constants::{
    DEFAULT_POSTGRES_DB, DEFAULT_POSTGRES_HOST, DEFAULT_POSTGRES_PASS, DEFAULT_POSTGRES_PORT,
    DEFAULT_POSTGRES_USER,
};

#[derive(Args, Debug, Clone)]
pub struct MigrateArgs {
    /// PostgreSQL server host
    #[arg(long, default_value = DEFAULT_POSTGRES_HOST)]
    pub host: String,

    /// PostgreSQL server port
    #[arg(long, default_value_t = DEFAULT_POSTGRES_PORT)]
    pub port: u16,

    /// PostgreSQL database user
    #[arg(long, default_value = DEFAULT_POSTGRES_USER)]
    pub user: String,

    /// PostgreSQL database password
    #[arg(long, default_value = DEFAULT_POSTGRES_PASS)]
    pub password: String,

    /// PostgreSQL database name
    #[arg(long, default_value = DEFAULT_POSTGRES_DB)]
    pub dbname: String,

    /// Check and display migration status without applying new migrations
    #[arg(long)]
    pub status: bool,
}

pub fn run_migrate(args: MigrateArgs) -> Result<()> {
    let mut client = DbClient::connect(
        &args.host,
        args.port,
        &args.user,
        &args.password,
        &args.dbname,
    )
    .with_context(|| {
        format!(
            "Failed to connect to PostgreSQL at {}:{} (db: {})",
            args.host, args.port, args.dbname
        )
    })?;

    if args.status {
        let applied = client.get_applied_migrations()?;
        println!("=== PostgreSQL Migration Status ===");
        println!(
            "Target Database: {}:{} ({})",
            args.host, args.port, args.dbname
        );
        if applied.is_empty() {
            println!("No migrations applied yet.");
        } else {
            for (version, name, applied_on) in applied {
                println!("  [V{}] {} (applied on {})", version, name, applied_on);
            }
        }
        return Ok(());
    }

    println!(
        "Applying pending refinery migrations on {}:{}...",
        args.host, args.port
    );
    let report = client.run_migrations()?;
    let applied = report.applied_migrations();
    if applied.is_empty() {
        println!("✓ Database schema is already up to date. (0 migrations applied)");
    } else {
        println!("✓ Successfully applied {} migration(s):", applied.len());
        for m in applied {
            println!("  + [V{}] {}", m.version(), m.name());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_args_defaults() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            migrate: MigrateArgs,
        }

        let cli = TestCli::parse_from(["test", "--host", "localhost", "--status"]);
        assert_eq!(cli.migrate.host, "localhost");
        assert_eq!(cli.migrate.port, DEFAULT_POSTGRES_PORT);
        assert!(cli.migrate.status);
    }
}
