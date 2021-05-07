use crate::{
    model::system::ModelSystem,
    sql::{column::Column, predicate::Predicate, select::Select, PhysicalTable, SQLOperation},
};

use crate::sql::order::OrderBy;

use crate::model::{operation::*, relation::*, types::*};

use super::{operation_context::OperationContext, Arguments};

use async_graphql_parser::{
    types::{Field, Selection, SelectionSet},
    Positioned,
};

use crate::execution::resolver::OutputName;

impl Query {
    pub fn resolve<'a>(
        &'a self,
        field: &'a Positioned<Field>,
        operation_context: &'a OperationContext<'a>,
    ) -> SQLOperation<'a> {
        SQLOperation::Select(self.operation(&field.node, Predicate::True, operation_context, true))
    }

    fn compute_order_by<'a>(
        &self,
        arguments: &'a Arguments,
        operation_context: &'a OperationContext<'a>,
    ) -> Option<OrderBy<'a>> {
        self.order_by_param.as_ref().and_then(|order_by_param| {
            let argument_value = super::find_arg(arguments, &order_by_param.name);
            argument_value.map(|argument_value| {
                order_by_param.compute_order_by(argument_value, operation_context)
            })
        })
    }

    fn operation<'a>(
        &'a self,
        field: &'a Field,
        additional_predicate: Predicate<'a>,
        operation_context: &'a OperationContext<'a>,
        top_level_selection: bool,
    ) -> Select<'a> {
        let table = self.physical_table(operation_context);

        let predicate = super::compute_predicate(
            &self.predicate_param,
            &field.arguments,
            additional_predicate,
            operation_context,
        );

        let content_object = self.content_select(&field.selection_set, operation_context);

        match self.return_type.type_modifier {
            ModelTypeModifier::Optional | ModelTypeModifier::NonNull => {
                let single_column = vec![content_object];
                table.select(single_column, predicate, None, top_level_selection)
            }
            ModelTypeModifier::List => {
                let order_by = self.compute_order_by(&field.arguments, operation_context);
                let agg_column = operation_context.create_column(Column::JsonAgg(content_object));
                let vector_column = vec![agg_column];
                table.select(vector_column, predicate, order_by, top_level_selection)
            }
        }
    }

    fn content_select<'a>(
        &self,
        selection_set: &'a Positioned<SelectionSet>,
        operation_context: &'a OperationContext<'a>,
    ) -> &'a Column<'a> {
        let column_specs: Vec<_> = selection_set
            .node
            .items
            .iter()
            .flat_map(|selection| self.map_selection(&selection.node, &operation_context))
            .collect();

        operation_context.create_column(Column::JsonObject(column_specs))
    }

    fn return_type<'a>(&self, system: &'a ModelSystem) -> &'a ModelType {
        let return_type_id = &self.return_type.type_id;
        &system.types[*return_type_id]
    }

    fn physical_table<'a>(&self, operation_context: &'a OperationContext<'a>) -> &'a PhysicalTable {
        let system = &operation_context.query_context.system;
        let return_type = self.return_type(system);
        match &return_type.kind {
            ModelTypeKind::Primitive => panic!(),
            ModelTypeKind::Composite {
                fields: _,
                table_id,
                ..
            } => &system.tables[*table_id],
        }
    }

    fn map_selection<'a>(
        &self,
        selection: &'a Selection,
        operation_context: &'a OperationContext<'a>,
    ) -> Vec<(String, &'a Column<'a>)> {
        match selection {
            Selection::Field(field) => {
                vec![self.map_field(&field.node, &operation_context)]
            }
            Selection::FragmentSpread(fragment_spread) => {
                let fragment_definition = operation_context
                    .query_context
                    .fragment_definition(&fragment_spread)
                    .unwrap();
                fragment_definition
                    .selection_set
                    .node
                    .items
                    .iter()
                    .flat_map(|selection| self.map_selection(&selection.node, &operation_context))
                    .collect()
            }
            Selection::InlineFragment(_inline_fragment) => {
                vec![] // TODO
            }
        }
    }

    fn map_field<'a>(
        &self,
        field: &'a Field,
        operation_context: &'a OperationContext<'a>,
    ) -> (String, &'a Column<'a>) {
        let system = operation_context.query_context.system;
        let return_type = self.return_type(system);

        let model_field = return_type.model_field(&field.name.node).unwrap();

        let column = match &model_field.relation {
            ModelRelation::Pk { column_id } | ModelRelation::Scalar { column_id } => {
                let column = column_id.get_column(system);
                Column::Physical(column)
            }
            ModelRelation::ManyToOne {
                column_id,
                other_type_id,
                ..
            } => {
                let other_type = &system.types[*other_type_id];
                let (other_table, other_table_pk_query) = {
                    match other_type.kind {
                        ModelTypeKind::Primitive => panic!(""),
                        ModelTypeKind::Composite {
                            table_id, pk_query, ..
                        } => (&system.tables[table_id], &system.queries[pk_query]),
                    }
                };

                Column::SelectionTableWrapper(
                    other_table.select(
                        vec![other_table_pk_query
                            .content_select(&field.selection_set, operation_context)],
                        Some(
                            operation_context.create_predicate(Predicate::Eq(
                                operation_context.create_column_with_id(column_id),
                                operation_context
                                    .create_column_with_id(&other_type.pk_column_id().unwrap()),
                            )),
                        ),
                        None,
                        false,
                    ),
                )
            }
            ModelRelation::OneToMany {
                other_type_column_id,
                other_type_id,
            } => {
                let other_type = &system.types[*other_type_id];
                let other_table_collection_query = {
                    match other_type.kind {
                        ModelTypeKind::Primitive => panic!(""),
                        ModelTypeKind::Composite {
                            collection_query, ..
                        } => &system.queries[collection_query],
                    }
                };

                let other_selection_table = other_table_collection_query.operation(
                    field,
                    Predicate::Eq(
                        operation_context.create_column_with_id(other_type_column_id),
                        operation_context
                            .create_column_with_id(&return_type.pk_column_id().unwrap()),
                    ),
                    operation_context,
                    false,
                );

                Column::SelectionTableWrapper(other_selection_table)
            }
        };

        (field.output_name(), operation_context.create_column(column))
    }
}
