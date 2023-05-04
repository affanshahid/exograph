// Copyright Exograph, Inc. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use anyhow::{anyhow, Result};
use colored::Colorize;

use core_plugin_interface::interface::SubsystemLoader;
use core_resolver::{context::RequestContext, system_resolver::SystemResolver, OperationsPayload};
use exo_deno::{deno_error::DenoError, Arg, DenoModule, DenoModuleSharedState, UserCode};
use exo_sql::{LOCAL_CONNECTION_POOL_SIZE, LOCAL_URL};
use include_dir::{include_dir, Dir};
use resolver::{create_system_resolver, LOCAL_ALLOW_INTROSPECTION};
use serde_json::Value;
use std::{collections::HashMap, path::Path};

use crate::exotest::common::TestResultKind;

use super::{
    common::TestResult,
    integration_tests::{run_query, MemoryRequest},
};

const INTROSPECTION_ASSERT_JS: &str = include_str!("introspection_tests.js");
const GRAPHQL_NODE_MODULE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/node_modules/graphql");

pub(crate) fn run_introspection_test(model_path: &Path) -> Result<TestResult> {
    let log_prefix = format!("(introspection: {})\n :: ", model_path.display()).purple();
    println!("{log_prefix} Running introspection tests...");

    let server = {
        let static_loaders: Vec<Box<dyn SubsystemLoader>> = vec![
            Box::new(postgres_resolver::PostgresSubsystemLoader {}),
            Box::new(deno_resolver::DenoSubsystemLoader {}),
        ];

        let base_name = model_path.to_str().unwrap();
        LOCAL_URL.with(|url| {
            url.borrow_mut()
                .replace("postgres://a@dummy-value".to_string());

            LOCAL_CONNECTION_POOL_SIZE.with(|pool_size| {
                pool_size.borrow_mut().replace(1);

                LOCAL_ALLOW_INTROSPECTION.with(|allow| {
                    allow.borrow_mut().replace(true);

                    create_system_resolver(&format!("{base_name}_ir"), static_loaders)
                })
            })
        })?
    };

    let result = check_introspection(&server)?;

    match result {
        Ok(()) => Ok(TestResult {
            log_prefix: log_prefix.to_string(),
            result: TestResultKind::Success,
        }),

        Err(e) => Ok(TestResult {
            log_prefix: log_prefix.to_string(),
            result: TestResultKind::Fail(e),
        }),
    }
}

fn check_introspection(server: &SystemResolver) -> Result<Result<()>> {
    let script = INTROSPECTION_ASSERT_JS;

    let deno_module_future = DenoModule::new(
        UserCode::LoadFromMemory {
            path: "internal/introspection_tests.js".to_owned(),
            script: script.into(),
        },
        "ExographTest",
        vec![],
        vec![],
        vec![],
        DenoModuleSharedState::default(),
        Some("Error"),
        Some(HashMap::from([(
            "graphql".to_string(),
            &GRAPHQL_NODE_MODULE,
        )])),
        Some(vec![(
            // TODO: move to a Rust-based solution
            // maybe juniper: https://github.com/graphql-rust/juniper/issues/217

            // We are currently importing the `graphql` NPM module used by graphiql and running it through Deno to perform schema validation
            // As it only depends on deno_core and deno_runtime, our integration of Deno does not include the NPM implementation provided through deno_cli
            // Therefore, we need to patch certain things in this module through extra_sources to get scripts to run in Deno

            // ReferenceError: process is not defined
            //    at embedded://graphql/jsutils/instanceOf.mjs:11:16
            "embedded://graphql/jsutils/instanceOf.mjs",
            GRAPHQL_NODE_MODULE
                .get_file("jsutils/instanceOf.mjs")
                .unwrap()
                .contents_utf8()
                .unwrap()
                .replace("process.env.NODE_ENV === 'production'", "false"),
        )]),
    );

    let runtime = tokio::runtime::Runtime::new()?;
    let mut deno_module = runtime.block_on(deno_module_future)?;

    let query = runtime.block_on(deno_module.execute_function("introspectionQuery", vec![]))?;

    let request = MemoryRequest::new(HashMap::new());
    let request_context = RequestContext::new(&request, vec![], server)?;
    let operations_payload = OperationsPayload {
        operation_name: None,
        query: if let Value::String(s) = query {
            s
        } else {
            panic!("expected string")
        },
        variables: None,
    };

    let result = run_query(
        &runtime,
        operations_payload,
        request_context,
        server,
        &mut HashMap::new(),
    );

    let result = runtime.block_on(deno_module.execute_function(
        "assertSchema",
        vec![Arg::Serde(Value::String(result.to_string()))],
    ));

    match result {
        Ok(_) => Ok(Ok(())),
        Err(e) => match e {
            DenoError::Explicit(e) => Ok(Err(anyhow!(e))),
            e => Err(anyhow!(e)),
        },
    }
}
