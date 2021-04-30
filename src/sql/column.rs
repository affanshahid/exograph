use super::{
    order::OrderBy, predicate::Predicate, table::*, Expression, ExpressionContext,
    ParameterBinding, SQLParam,
};

#[derive(Debug, Clone)]
pub struct PhysicalColumn {
    pub table_name: String,
    pub column_name: String,
}

#[derive(Debug)]
pub enum Column<'a> {
    Physical(&'a PhysicalColumn),
    Literal(Box<dyn SQLParam>),
    JsonObject(Vec<(String, &'a Column<'a>)>),
    JsonAgg(&'a Column<'a>),
    SingleSelect {
        table: &'a PhysicalTable,
        column: &'a Column<'a>,
        predicate: Option<&'a Predicate<'a>>,
        order_by: Option<OrderBy<'a>>,
    },
    SelectionTableWrapper(SelectionTable<'a>),
}

impl<'a> Expression for Column<'a> {
    fn binding(&self, expression_context: &mut ExpressionContext) -> ParameterBinding {
        match self {
            Column::Physical(PhysicalColumn {
                table_name,
                column_name,
            }) => ParameterBinding::new(format!("\"{}\".\"{}\"", table_name, column_name), vec![]),
            Column::Literal(value) => {
                let param_index = expression_context.next_param();
                ParameterBinding::new(format! {"${}", param_index}, vec![value])
            }
            Column::JsonObject(elems) => {
                let (elem_stmt, elem_params): (Vec<_>, Vec<_>) = elems
                    .iter()
                    .map(|elem| {
                        let elem_binding = elem.1.binding(expression_context);
                        (
                            format!("'{}', {}", elem.0, elem_binding.stmt),
                            elem_binding.params,
                        )
                    })
                    .unzip();

                let stmt = format!("json_build_object({})", elem_stmt.join(", "));
                let params = elem_params.into_iter().flatten().collect();
                ParameterBinding::new(stmt, params)
            }
            Column::JsonAgg(column) => {
                // coalesce to return an empty array if we have no matching enities
                let column_binding = column.binding(expression_context);
                let stmt = format!("coalesce(json_agg({}), '[]'::json)", column_binding.stmt);
                ParameterBinding::new(stmt, column_binding.params)
            }
            Column::SingleSelect {
                table,
                column,
                predicate,
                order_by,
            } => {
                let column_binding = column.binding(expression_context);
                let table_binding = table.binding(expression_context);
                let predicate_binding = predicate.unwrap().binding(expression_context);
                let stmt = format!(
                    "(select {} from {} where {})",
                    column_binding.stmt, table_binding.stmt, predicate_binding.stmt
                );
                ParameterBinding::new(stmt, column_binding.params)
            }
            Column::SelectionTableWrapper(selection_table) => {
                let pb = selection_table.binding(expression_context);
                ParameterBinding::new(format!("({})", pb.stmt), pb.params)
            }
        }
    }
}
