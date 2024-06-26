// Copyright Exograph, Inc. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::io::BufRead;

use super::{
    docker::DockerPostgresDatabaseServer, error::EphemeralDatabaseSetupError,
    local::LocalPostgresDatabaseServer,
};

/// Launcher for an ephemeral database server using either a local Postgres installation or Docker
pub struct EphemeralDatabaseLauncher {}

impl EphemeralDatabaseLauncher {
    /// Create a new database server.
    /// Currently, it prefers a local installation, but falls back to Docker if it's not available
    pub fn create_server(
    ) -> Result<Box<dyn EphemeralDatabaseServer + Send + Sync>, EphemeralDatabaseSetupError> {
        let local_res = LocalPostgresDatabaseServer::check_availability();
        if local_res.is_ok() {
            println!("Launching PostgreSQL locally...");
            LocalPostgresDatabaseServer::start()
        } else {
            eprintln!(
                "Local PostgreSQL is not available: {}",
                local_res.unwrap_err()
            );

            let docker_res = DockerPostgresDatabaseServer::check_availability();
            if docker_res.is_ok() {
                println!("Launching PostgreSQL in Docker...");
                DockerPostgresDatabaseServer::start()
            } else {
                eprintln!(
                    "Docker PostgreSQL is not available: {}",
                    docker_res.unwrap_err()
                );
                Err(EphemeralDatabaseSetupError::Generic(
                    "Neither local PostgreSQL nor Docker is available".to_string(),
                ))
            }
        }
    }
}

/// A ephemeral database server that can create ephemeral databases
/// Implemented should implement `Drop` to clean up the server to free up resources
pub trait EphemeralDatabaseServer {
    /// Create a new database on the server with the specified name
    fn create_database(
        &self,
        name: &str,
    ) -> Result<Box<dyn EphemeralDatabase + Send + Sync>, EphemeralDatabaseSetupError>;
}

/// A ephemeral database that can be used for testing.
/// Implemented should implement `Drop` to clean up the database to free up resources
pub trait EphemeralDatabase {
    /// Get the URL to connect to the database. The URL should be in the format suitable as the `psql` argument
    fn url(&self) -> String;
}

/// A utility function to launch a process and wait for it to exit
pub(super) fn launch_process(
    name: &str,
    args: &[&str],
    report_errors: bool,
) -> Result<(), EphemeralDatabaseSetupError> {
    let mut command = std::process::Command::new(name)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            EphemeralDatabaseSetupError::Generic(format!("Failed to launch process {name}: {e}"))
        })?;

    let status = command.wait().map_err(|e| {
        EphemeralDatabaseSetupError::Generic(format!("Failed to wait for process {name}: {e}"))
    })?;

    if !status.success() {
        if report_errors {
            if let Some(stderr) = command.stderr.take() {
                let stderr = std::io::BufReader::new(stderr);
                stderr.lines().for_each(|line| {
                    eprintln!("{}: {}", name, line.unwrap());
                });
            }
        }
        return Err(EphemeralDatabaseSetupError::Generic(format!(
            "Process {name} exited with non-zero status code {status}",
        )));
    }
    Ok(())
}
