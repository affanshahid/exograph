// Copyright Exograph, Inc. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::pin::Pin;
use std::process::exit;
use std::{fs::File, io::BufReader, path::Path};

use crate::system_loader::{StaticLoaders, SystemLoadingError};

#[cfg(not(target_family = "wasm"))]
use common::env_const::is_production;
use core_plugin_shared::serializable_system::SerializableSystem;
use core_plugin_shared::trusted_documents::TrustedDocumentEnforcement;
use core_resolver::QueryResponse;

use super::system_loader::SystemLoader;
use ::tracing::instrument;
use async_graphql_parser::Pos;
use async_stream::try_stream;
use bytes::Bytes;
use core_resolver::system_resolver::SystemResolutionError;
use core_resolver::system_resolver::SystemResolver;
pub use core_resolver::OperationsPayload;
use core_resolver::{context::RequestContext, QueryResponseBody};
use futures::Stream;

use exo_env::Environment;

const EXO_PLAYGROUND_HTTP_PATH: &str = "EXO_PLAYGROUND_HTTP_PATH";
const EXO_ENDPOINT_HTTP_PATH: &str = "EXO_ENDPOINT_HTTP_PATH";

#[instrument(
    name = "resolver::resolve_in_memory"
    skip(system_resolver, request_context)
)]
pub async fn resolve_in_memory<'a>(
    operations_payload: OperationsPayload,
    system_resolver: &SystemResolver,
    request_context: RequestContext<'a>,
    trusted_document_enforcement: TrustedDocumentEnforcement,
) -> Result<Vec<(String, QueryResponse)>, SystemResolutionError> {
    let response = system_resolver
        .resolve_operations(
            operations_payload,
            &request_context,
            trusted_document_enforcement,
        )
        .await;

    let ctx = request_context.get_base_context();
    let mut tx_holder = ctx.transaction_holder.try_lock().unwrap();

    tx_holder
        .finalize(response.is_ok())
        .await
        .map_err(|e| {
            SystemResolutionError::Generic(format!("Error while finalizing transaction: {e}"))
        })
        .and(response)
}

pub type Headers = Vec<(String, String)>;
pub type ResponseStream<E> = (Pin<Box<dyn Stream<Item = Result<Bytes, E>>>>, Headers);
/// Resolves an incoming query, returning a response stream containing JSON and a set
/// of HTTP headers. The JSON may be either the data returned by the query, or a list of errors
/// if something went wrong.
///
/// In a typical use case (for example server-actix), the caller will
/// first call `create_system_resolver_or_exit` to create a [SystemResolver] object, and
/// then call `resolve` with that object.
#[instrument(
    name = "resolver::resolve"
    skip(system_resolver, request_context)
)]
pub async fn resolve<'a, E: 'static>(
    operations_payload: OperationsPayload,
    system_resolver: &SystemResolver,
    request_context: RequestContext<'a>,
    playground_request: bool,
) -> ResponseStream<E> {
    #[cfg(not(target_family = "wasm"))]
    let is_production = is_production();
    #[cfg(target_family = "wasm")]
    let is_production = !playground_request;

    let response = resolve_in_memory(
        operations_payload,
        system_resolver,
        request_context,
        if playground_request && !is_production {
            TrustedDocumentEnforcement::DoNotEnforce
        } else {
            TrustedDocumentEnforcement::Enforce
        },
    )
    .await;

    let headers = if let Ok(ref response) = response {
        response
            .iter()
            .flat_map(|(_, qr)| qr.headers.clone())
            .collect()
    } else {
        vec![]
    };

    let stream = try_stream! {
        macro_rules! report_position {
            ($position:expr) => {
                let p: Pos = $position;

                yield Bytes::from_static(br#"{"line": "#);
                yield Bytes::from(p.line.to_string());
                yield Bytes::from_static(br#", "column": "#);
                yield Bytes::from(p.column.to_string());
                yield Bytes::from_static(br#"}"#);
            };
        }

        macro_rules! report_positions {
            ($positions:expr) => {
                let mut first = true;
                for p in $positions {
                    if !first {
                        yield Bytes::from_static(b", ");
                    }
                    first = false;
                    report_position!(p);
                }
            };
        }

        match response {
            Ok(parts) => {
                let parts_len = parts.len();
                yield Bytes::from_static(br#"{"data": {"#);
                for (index, part) in parts.into_iter().enumerate() {
                    yield Bytes::from_static(b"\"");
                    yield Bytes::from(part.0);
                    yield Bytes::from_static(br#"":"#);
                    match part.1.body {
                        QueryResponseBody::Json(value) => yield Bytes::from(value.to_string()),
                        QueryResponseBody::Raw(Some(value)) => yield Bytes::from(value),
                        QueryResponseBody::Raw(None) => yield Bytes::from_static(b"null"),
                    };
                    if index != parts_len - 1 {
                        yield Bytes::from_static(b", ");
                    }
                };
                yield Bytes::from_static(b"}}");
            },
            Err(err) => {
                yield Bytes::from_static(br#"{"errors": [{"message":""#);
                yield Bytes::from(
                    err.user_error_message().to_string()
                        .replace('\"', "")
                        .replace('\n', "; ")
                );
                yield Bytes::from_static(br#"""#);
                if let SystemResolutionError::Validation(err) = err {
                    yield Bytes::from_static(br#", "locations": ["#);
                    report_positions!(err.positions());
                    yield Bytes::from_static(br#"]"#);
                };
                yield Bytes::from_static(br#"}"#);
                yield Bytes::from_static(b"]}");
            },
        }
    };

    let boxed_stream = Box::pin(stream) as Pin<Box<dyn Stream<Item = Result<Bytes, E>>>>;

    (boxed_stream, headers)
}

pub fn get_playground_http_path() -> String {
    std::env::var(EXO_PLAYGROUND_HTTP_PATH).unwrap_or_else(|_| "/playground".to_string())
}

pub fn get_endpoint_http_path() -> String {
    std::env::var(EXO_ENDPOINT_HTTP_PATH).unwrap_or_else(|_| "/graphql".to_string())
}

pub async fn create_system_resolver(
    exo_ir_file: &str,
    static_loaders: StaticLoaders,
    env: Box<dyn Environment>,
) -> Result<SystemResolver, SystemLoadingError> {
    if !Path::new(&exo_ir_file).exists() {
        return Err(SystemLoadingError::FileNotFound(exo_ir_file.to_string()));
    }
    match File::open(exo_ir_file) {
        Ok(file) => {
            let exo_ir_file_buffer = BufReader::new(file);

            SystemLoader::load(exo_ir_file_buffer, static_loaders, env).await
        }
        Err(e) => Err(SystemLoadingError::FileOpen(exo_ir_file.into(), e)),
    }
}

pub async fn create_system_resolver_from_system(
    system: SerializableSystem,
    static_loaders: StaticLoaders,
    env: Box<dyn Environment>,
) -> Result<SystemResolver, SystemLoadingError> {
    SystemLoader::load_from_system(system, static_loaders, env).await
}

pub async fn create_system_resolver_or_exit(
    exo_ir_file: &str,
    static_loaders: StaticLoaders,
    env: Box<dyn Environment>,
) -> SystemResolver {
    match create_system_resolver(exo_ir_file, static_loaders, env).await {
        Ok(system_resolver) => system_resolver,
        Err(error) => {
            println!("{error}");
            exit(1);
        }
    }
}
