// Copyright Exograph, Inc. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Dynamic loader for postgres-resolver.

use core_plugin_interface::interface::SubsystemLoader;
use postgres_resolver::PostgresSubsystemLoader;

// Currently, Rust doesn't allow cfg attributes on crate-type (see
// https://github.com/rust-lang/rust/issues/20267). If it did, we wouldn't need this crate. Instead,
// we could have set the `crate-type` of "postgres-resolver" to ["lib", "cdylib"] and put
// `#[cfg(crate_type="dylib")]` on the `export_subsystem_loader!` macro invocation. So as a
// workaround, we use this crate whose `crate-type` is "cdynlib" (whereas for the postgres-resolver
// crate, it's "lib").
//
// Also, see the caution in the comment for `export_subsystem_loader!`

core_plugin_interface::export_subsystem_loader!(PostgresSubsystemLoader {
    existing_client: None,
});
