use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use maybe_owned::MaybeOwned;
use payas_deno::Arg;
use postgres::{types::FromSqlOwned, Row};
use serde_json::Map;

use crate::execution::query_context::{QueryContext, QueryResponse};

use super::access_solver;
use super::interception::InterceptedOperation;
use crate::execution::resolver::{FieldResolver, GraphQLExecutionError};
use async_graphql_parser::{types::Field, Positioned};
use async_graphql_value::ConstValue;
use payas_model::{
    model::{
        mapped_arena::SerializableSlabIndex,
        operation::{Interceptors, Mutation, OperationReturnType},
        service::{ServiceMethod, ServiceMethodType},
        GqlCompositeType, GqlCompositeTypeKind, GqlTypeKind,
    },
    sql::{predicate::Predicate, transaction::TransactionScript, Select},
};

pub trait SQLMapper<'a, R> {
    fn map_to_sql(
        &'a self,
        argument: &'a ConstValue,
        query_context: &'a QueryContext<'a>,
    ) -> Result<R>;
}

pub trait SQLUpdateMapper<'a> {
    fn update_script(
        &'a self,
        mutation: &'a Mutation,
        predicate: MaybeOwned<'a, Predicate<'a>>,
        select: Select<'a>,
        argument: &'a ConstValue,
        query_context: &'a QueryContext<'a>,
    ) -> Result<TransactionScript>;
}
pub trait OperationResolver<'a> {
    fn resolve_operation(
        &'a self,
        field: &'a Positioned<Field>,
        query_context: &'a QueryContext<'a>,
    ) -> Result<OperationResolverResult<'a>>;

    fn execute(
        &'a self,
        field: &'a Positioned<Field>,
        query_context: &'a QueryContext<'a>,
    ) -> Result<QueryResponse> {
        let resolver_result = self.resolve_operation(field, query_context)?;
        let interceptors = self.interceptors().ordered();

        let op_name = &self.name();

        let intercepted_operation =
            InterceptedOperation::new(op_name, resolver_result, interceptors);
        intercepted_operation.execute(field, query_context)
    }

    fn name(&self) -> &str;

    fn interceptors(&self) -> &Interceptors;
}

pub enum OperationResolverResult<'a> {
    SQLOperation(TransactionScript<'a>),
    DenoOperation(SerializableSlabIndex<ServiceMethod>),
}

pub enum SQLOperationKind {
    Create,
    Retrieve,
    Update,
    Delete,
}

pub fn compute_sql_access_predicate<'a>(
    return_type: &OperationReturnType,
    kind: &SQLOperationKind,
    query_context: &'a QueryContext<'a>,
) -> Predicate<'a> {
    let return_type = return_type.typ(query_context.get_system());

    match &return_type.kind {
        GqlTypeKind::Primitive => Predicate::True,
        GqlTypeKind::Composite(GqlCompositeType { access, .. }) => {
            let access_expr = match kind {
                SQLOperationKind::Create => &access.creation,
                SQLOperationKind::Retrieve => &access.read,
                SQLOperationKind::Update => &access.update,
                SQLOperationKind::Delete => &access.delete,
            };
            access_solver::reduce_access(access_expr, query_context.request_context, query_context)
        }
    }
}

pub fn compute_service_access_predicate<'a>(
    return_type: &OperationReturnType,
    method: &'a ServiceMethod,
    query_context: &'a QueryContext<'a>,
) -> &'a Predicate<'a> {
    let return_type = return_type.typ(query_context.get_system());

    let type_level_access = match &return_type.kind {
        GqlTypeKind::Primitive => Predicate::True,
        GqlTypeKind::Composite(GqlCompositeType {
            access,
            kind: GqlCompositeTypeKind::NonPersistent,
            ..
        }) => {
            let access_expr = match &method.operation_kind {
                ServiceMethodType::Query(_) => &access.read, // query
                ServiceMethodType::Mutation(_) => &access.creation, // mutation
            };
            access_solver::reduce_access(access_expr, query_context.request_context, query_context)
        }
        _ => panic!(),
    };

    let method_access_expr = match &method.operation_kind {
        ServiceMethodType::Query(_) => &method.access.read, // query
        ServiceMethodType::Mutation(_) => &method.access.creation, // mutation
    };

    let method_level_access = access_solver::reduce_access(
        method_access_expr,
        query_context.request_context,
        query_context,
    );

    if matches!(type_level_access, Predicate::False)
        || matches!(method_level_access, Predicate::False)
    {
        &Predicate::False // deny if either access check fails
    } else {
        &Predicate::True
    }
}

impl<'a> OperationResolverResult<'a> {
    pub fn execute(
        &self,
        field: &Positioned<Field>,
        query_context: &'a QueryContext<'a>,
    ) -> Result<QueryResponse> {
        match self {
            OperationResolverResult::SQLOperation(transaction_script) => {
                let mut client = query_context.executor.database.get_client()?;
                let mut result = transaction_script.execute(&mut client, extractor)?;

                if result.len() == 1 {
                    Ok(QueryResponse::Raw(Some(result.swap_remove(0))))
                } else if result.is_empty() {
                    Ok(QueryResponse::Raw(None))
                } else {
                    bail!(format!(
                        "Result has {} entries; expected only zero or one",
                        result.len()
                    ))
                }
            }

            OperationResolverResult::DenoOperation(method_id) => {
                let method = &query_context.executor.system.methods[*method_id];

                let access_predicate =
                    compute_service_access_predicate(&method.return_type, method, query_context);

                if access_predicate == &Predicate::False {
                    bail!(anyhow!(GraphQLExecutionError::Authorization))
                }

                resolve_deno(method, field, query_context).map(QueryResponse::Json)
            }
        }
    }
}

fn resolve_deno(
    method: &ServiceMethod,
    field: &Positioned<Field>,
    query_context: &QueryContext<'_>,
) -> Result<serde_json::Value> {
    let path = &method.module_path;

    let function_result = futures::executor::block_on(async {
        let mapped_args = query_context
            .field_arguments(&field.node)?
            .iter()
            .map(|(gql_name, gql_value)| {
                (
                    gql_name.node.as_str().to_owned(),
                    gql_value.node.clone().into_json().unwrap(),
                )
            })
            .collect::<HashMap<_, _>>();

        let arg_sequence = method
            .arguments
            .iter()
            .map(|arg| {
                let arg_type = &query_context.executor.system.types[arg.type_id];

                if arg.is_injected {
                    Ok(Arg::Shim(arg_type.name.clone()))
                } else if let Some(val) = mapped_args.get(&arg.name) {
                    Ok(Arg::Serde(val.clone()))
                } else {
                    Err(anyhow!("Invalid argument {}", arg.name))
                }
            })
            .collect::<Result<Vec<_>>>()?;

        query_context.executor.deno_execution.execute_function(
            path,
            &method.name,
            arg_sequence,
            &|query_string, variables| {
                let result = query_context
                    .executor
                    .execute_with_request_context(
                        None,
                        &query_string,
                        variables,
                        query_context.request_context.clone(),
                    )?
                    .into_iter()
                    .map(|(name, response)| (name, response.to_json().unwrap()))
                    .collect::<Map<_, _>>();

                Ok(serde_json::Value::Object(result))
            },
            None,
            None,
        )
    })?;

    let result = if let serde_json::Value::Object(_) = function_result {
        let resolved_set =
            function_result.resolve_selection_set(query_context, &field.node.selection_set)?;
        serde_json::Value::Object(resolved_set.into_iter().collect())
    } else {
        function_result
    };

    Ok(result)
}

fn extractor<T: FromSqlOwned>(row: Row) -> Result<T> {
    match row.try_get(0) {
        Ok(col) => Ok(col),
        Err(err) => bail!("Got row without any columns {}", err),
    }
}

// TODO: Define this so that we can use it from multiple ways to invoke (service method and interceptors)
// fn execute_query_fn<'a>(
//     query_context: &'a QueryContext<'a>,
// ) -> &'a dyn Fn(
//     String,
//     Option<&serde_json::Map<String, serde_json::Value>>,
// ) -> Result<serde_json::Value> {
//     &|query_string: String, variables| {
//         let result = query_context
//             .executor
//             .execute_with_request_context(
//                 None,
//                 &query_string,
//                 variables,
//                 query_context.request_context.clone(),
//             )?
//             .into_iter()
//             .map(|(name, response)| (name, response.to_json().unwrap()))
//             .collect::<Map<_, _>>();

//         Ok(serde_json::Value::Object(result))
//     }
// }