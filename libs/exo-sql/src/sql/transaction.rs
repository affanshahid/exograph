use tokio_postgres::{GenericClient, Row, Transaction};
use tracing::{debug, error, instrument};

use crate::{database_error::DatabaseError, sql::SQLBuilder};

use super::{
    sql_operation::{SQLOperation, TemplateSQLOperation},
    ExpressionBuilder, SQLValue,
};

/// Rows obtained from a SQL operation
pub type TransactionStepResult = Vec<Row>;

/// Sequence of SQL operations that are executed in a transaction
#[derive(Default, Debug)]
pub struct TransactionScript<'a> {
    steps: Vec<TransactionStep<'a>>,
}

/// Collection of results from steps in a transaction
pub struct TransactionContext {
    results: Vec<TransactionStepResult>,
}

#[derive(Debug, Clone, Copy)]
pub struct TransactionStepId(pub usize);

impl TransactionContext {
    /// Returns the value of a column in a row from the given step id
    pub fn resolve_value(&self, step_id: TransactionStepId, row: usize, col: usize) -> SQLValue {
        self.results[step_id.0][row].get::<usize, SQLValue>(col)
    }

    /// Returns the number of rows in the result of the given step id
    pub fn row_count(&self, step_id: TransactionStepId) -> usize {
        self.results[step_id.0].len()
    }
}

impl<'a> TransactionScript<'a> {
    /// Returns the result of the last step
    #[instrument(
        name = "TransactionScript::execute"
        skip(self, tx)
        )]
    pub async fn execute(
        &self,
        tx: &mut Transaction<'_>,
    ) -> Result<TransactionStepResult, DatabaseError> {
        let mut transaction_context = TransactionContext { results: vec![] };

        // Execute each step in the transaction and store the result in the transaction_context
        for step in self.steps.iter() {
            let result = step.execute(tx, &transaction_context).await?;
            transaction_context.results.push(result)
        }

        // Return the result of the last step (usually the "select")
        transaction_context
            .results
            .into_iter()
            .last()
            .ok_or_else(|| DatabaseError::Transaction("".into()))
    }

    /// Adds a step to the transaction script and return the step id (which is just the index of the step in the script)
    pub fn add_step(&mut self, step: TransactionStep<'a>) -> TransactionStepId {
        let id = self.steps.len();
        self.steps.push(step);
        TransactionStepId(id)
    }
}

#[derive(Debug)]
pub enum TransactionStep<'a> {
    Concrete(ConcreteTransactionStep<'a>),
    Template(TemplateTransactionStep<'a>),
}

impl<'a> TransactionStep<'a> {
    #[instrument(
        name = "TransactionStep::execute"
        level = "trace"
        skip(self, client, transaction_context)
        )]
    pub async fn execute(
        &self,
        client: &mut impl GenericClient,
        transaction_context: &TransactionContext,
    ) -> Result<TransactionStepResult, DatabaseError> {
        match self {
            Self::Concrete(step) => step.execute(client).await,
            Self::Template(step) => {
                let concrete = step.resolve(transaction_context);

                match concrete.as_slice() {
                    [init @ .., last] => {
                        // Execute all but the last step
                        for substep in init {
                            substep.execute(client).await?;
                        }
                        // Execute the last step and return the result
                        last.execute(client).await
                    }
                    _ => Ok(vec![]),
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct ConcreteTransactionStep<'a> {
    pub operation: SQLOperation<'a>,
}

impl<'a> ConcreteTransactionStep<'a> {
    pub fn new(operation: SQLOperation<'a>) -> Self {
        Self { operation }
    }

    #[instrument(
        name = "ConcreteTransactionStep::execute"
        level = "trace"
        skip(self, client)
        fields(
            operation = ?self.operation
            )
        )]
    pub async fn execute(
        &'a self,
        client: &mut impl GenericClient,
    ) -> Result<TransactionStepResult, DatabaseError> {
        self.run_query(client).await
    }

    async fn run_query(
        &'a self,
        client: &mut impl GenericClient,
    ) -> Result<Vec<Row>, DatabaseError> {
        let mut sql_builder = SQLBuilder::new();
        self.operation.build(&mut sql_builder);
        let (stmt, params) = sql_builder.into_sql();

        let params: Vec<_> = params.iter().map(|p| p.as_pg()).collect();

        debug!("Executing SQL operation: {}", stmt);

        client.query(&stmt, &params[..]).await.map_err(|e| {
            error!("Failed to execute query: {e:?}");
            DatabaseError::Delegate(e).with_context("Database operation failed".into())
        })
    }
}

#[derive(Debug)]
pub struct TemplateTransactionStep<'a> {
    pub operation: TemplateSQLOperation<'a>,
    pub prev_step_id: TransactionStepId,
}

impl<'a> TemplateTransactionStep<'a> {
    pub fn resolve(
        &'a self,
        transaction_context: &TransactionContext,
    ) -> Vec<ConcreteTransactionStep<'a>> {
        self.operation
            .resolve(self.prev_step_id, transaction_context)
            .into_iter()
            .map(|operation| ConcreteTransactionStep { operation })
            .collect()
    }
}