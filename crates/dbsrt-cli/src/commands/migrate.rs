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
    use crate::constants::{STANDARD_POSTGRES_PORT, TEST_UNREACHABLE_PORT};

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

    #[test]
    fn test_migrate_connection_failure() {
        let args = MigrateArgs {
            host: DEFAULT_POSTGRES_HOST.to_string(),
            port: TEST_UNREACHABLE_PORT,
            user: DEFAULT_POSTGRES_USER.to_string(),
            password: DEFAULT_POSTGRES_PASS.to_string(),
            dbname: DEFAULT_POSTGRES_DB.to_string(),
            status: false,
        };
        let err = run_migrate(args).unwrap_err();
        assert!(err.to_string().contains("Failed to connect to PostgreSQL"));
    }

    #[test]
    fn test_migrate_live_db_lifecycle() {
        let _guard = crate::commands::LIVE_DB_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let host = std::env::var("PSQL_IP").unwrap_or_else(|_| DEFAULT_POSTGRES_HOST.to_string());
        let env_port = std::env::var("PSQL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok());
        let ports: Vec<u16> = match env_port {
            Some(p) => vec![p, DEFAULT_POSTGRES_PORT, STANDARD_POSTGRES_PORT],
            None => vec![DEFAULT_POSTGRES_PORT, STANDARD_POSTGRES_PORT],
        };

        let mut reachable_port = None;
        for port in ports {
            if let Ok(mut client) = DbClient::connect(
                &host,
                port,
                DEFAULT_POSTGRES_USER,
                DEFAULT_POSTGRES_PASS,
                DEFAULT_POSTGRES_DB,
            ) {
                let _ = client.init_schema();
                reachable_port = Some(port);
                break;
            }
        }

        let Some(port) = reachable_port else {
            eprintln!("PostgreSQL not reachable, skipping live DB test_migrate_live_db_lifecycle");
            return;
        };

        // 1. Inspect status
        let args_status = MigrateArgs {
            host: host.clone(),
            port,
            user: DEFAULT_POSTGRES_USER.to_string(),
            password: DEFAULT_POSTGRES_PASS.to_string(),
            dbname: DEFAULT_POSTGRES_DB.to_string(),
            status: true,
        };
        assert!(run_migrate(args_status).is_ok());

        // 2. Apply migrations (idempotent)
        let args_apply = MigrateArgs {
            host,
            port,
            user: DEFAULT_POSTGRES_USER.to_string(),
            password: DEFAULT_POSTGRES_PASS.to_string(),
            dbname: DEFAULT_POSTGRES_DB.to_string(),
            status: false,
        };
        assert!(run_migrate(args_apply).is_ok());
    }
}
