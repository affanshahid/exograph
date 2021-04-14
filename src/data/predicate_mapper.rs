use crate::sql::{column::Column, predicate::Predicate};

use crate::model::predicate::*;

use async_graphql_value::Value;

use super::operation_context::OperationContext;

impl PredicateParameter {
    pub fn compute_predicate<'a>(
        &self,
        argument_value: &'a Value,
        operation_context: &'a OperationContext<'a>,
    ) -> Predicate<'a> {
        let system = operation_context.query_context.system;
        let parameter_type = system.predicate_types.get_by_id(self.type_id).unwrap();

        match &parameter_type.kind {
            PredicateParameterTypeKind::ImplicitEqual => Predicate::Eq(
                operation_context.create_column(Column::Physical(
                    &self
                        .column_id
                        .as_ref()
                        .and_then(|column_id| column_id.get_column(system))
                        .unwrap(),
                )),
                operation_context.literal_column(argument_value),
            ),
            PredicateParameterTypeKind::Opeartor(parameters) => {
                parameters.iter().fold(Predicate::True, |acc, parameter| {
                    let new_predicate =
                        match Self::get_argument_value_component(argument_value, &parameter.name) {
                            Some(op_value) => {
                                self.op_predicate(&parameter.name, op_value, operation_context)
                            }
                            None => Predicate::True,
                        };

                    Predicate::And(Box::new(acc), Box::new(new_predicate))
                })
            }
            PredicateParameterTypeKind::Composite(parameters) => {
                parameters.iter().fold(Predicate::True, |acc, parameter| {
                    let new_predicate =
                        match Self::get_argument_value_component(argument_value, &parameter.name) {
                            Some(argument_value_component) => parameter
                                .compute_predicate(argument_value_component, operation_context),
                            None => Predicate::True,
                        };

                    Predicate::And(Box::new(acc), Box::new(new_predicate))
                })
            }
        }
    }

    fn op_predicate<'a>(
        &self,
        op_name: &str,
        op_value: &'a Value,
        operation_context: &'a OperationContext<'a>,
    ) -> Predicate<'a> {
        let op_key_column = operation_context.create_column(Column::Physical(
            &self
                .column_id
                .as_ref()
                .and_then(|column_id| column_id.get_column(operation_context.query_context.system))
                .unwrap(),
        ));
        let op_value_column = operation_context.literal_column(op_value);
        Predicate::from_name(op_name, op_key_column, op_value_column)
    }

    fn get_argument_value_component<'a>(
        argument_value: &'a Value,
        component_name: &str,
    ) -> Option<&'a Value> {
        match argument_value {
            Value::Object(value) => value.get(component_name),
            _ => None,
        }
    }
}
