use std::{io, path::PathBuf, time::SystemTime};

use crate::{commands::command::Command, util::open_file_for_output};

use super::migration_helper::migration_statements;
use anyhow::Result;
use payas_sql::{schema::spec::SchemaSpec, Database};

/// Perform a database migration for a claytip model
pub struct MigrateCommand {
    pub model: PathBuf,
    pub comment_destructive_changes: bool,
    pub output: Option<PathBuf>,
}

impl Command for MigrateCommand {
    fn run(&self, _system_start_time: Option<SystemTime>) -> Result<()> {
        let mut buffer: Box<dyn io::Write> = if let Some(output) = &self.output {
            Box::new(open_file_for_output(output)?)
        } else {
            Box::new(io::stdout())
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();

        rt.block_on(async {
            let database = Database::from_env(Some(1))?; // TODO: error handling here
            let client = database.get_client().await?;

            let old_schema = SchemaSpec::from_db(&client).await?;

            for issue in &old_schema.issues {
                println!("{}", issue);
            }

            let new_system = payas_parser::build_system(&self.model)?;
            let new_schema = SchemaSpec::from_model(new_system.tables.into_iter().collect());

            let statements = migration_statements(&old_schema.value, &new_schema);

            for (statement, is_destructive) in statements {
                if is_destructive && self.comment_destructive_changes {
                    write!(buffer, "-- ")?;
                }
                write!(buffer, "{}", statement)?;
            }

            Ok(())
        })
    }
}