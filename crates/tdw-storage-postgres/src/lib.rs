#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod sqlx_engine;

#[cfg(feature = "postgres")]
pub use sqlx_engine::PgEngine;

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tdw_core::{
    DataModel, Error, HealthStatus, OBBject, RelationalEngine, Result, WriteReceipt, WriteSink,
};

#[derive(Debug, Default)]
pub struct PostgresRecordingEngine {
    statements: Mutex<Vec<String>>,
    rows_written: Mutex<usize>,
}

impl PostgresRecordingEngine {
    pub fn statements(&self) -> Result<Vec<String>> {
        self.statements
            .lock()
            .map(|statements| statements.clone())
            .map_err(|error| Error::Storage(error.to_string()))
    }

    pub fn rows_written(&self) -> Result<usize> {
        self.rows_written
            .lock()
            .map(|rows| *rows)
            .map_err(|error| Error::Storage(error.to_string()))
    }
}

#[async_trait]
impl RelationalEngine for PostgresRecordingEngine {
    async fn execute(&self, sql: &str, _params: Value) -> Result<u64> {
        validate_sql(sql)?;
        self.statements
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?
            .push(sql.to_string());
        Ok(1)
    }

    async fn fetch_json(&self, sql: &str, params: Value) -> Result<Vec<Value>> {
        validate_sql(sql)?;
        Ok(vec![serde_json::json!({
            "engine": "postgres-recording",
            "sql": sql,
            "params": params
        })])
    }
}

fn validate_sql(sql: &str) -> Result<()> {
    if sql.trim().is_empty() {
        return Err(Error::Storage("postgres sql must not be empty".to_string()));
    }
    Ok(())
}

#[async_trait]
impl<T: DataModel> WriteSink<T> for PostgresRecordingEngine {
    fn name(&self) -> &'static str {
        "postgres-recording"
    }

    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
        let mut rows = self
            .rows_written
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        *rows += batch.rows.len();
        Ok(WriteReceipt {
            sink: "postgres-recording",
            rows_written: batch.rows.len(),
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Row {
        symbol: String,
    }

    #[test]
    fn exposes_relational_and_write_sink_contracts() {
        fn assert_relational<T: RelationalEngine>() {}
        fn assert_sink<T: WriteSink<Row>>() {}

        assert_relational::<PostgresRecordingEngine>();
        assert_sink::<PostgresRecordingEngine>();
    }

    #[test]
    fn records_sql_fetches_rows_and_health() {
        let engine = PostgresRecordingEngine::default();
        let affected = block_on_ready(engine.execute("insert into bars", serde_json::json!({})))
            .unwrap_or_else(|error| panic!("execute should succeed: {error}"));
        let rows = block_on_ready(engine.fetch_json("select * from bars", serde_json::json!([1])))
            .unwrap_or_else(|error| panic!("fetch should succeed: {error}"));
        let batch = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "fixture",
            "rows",
        );
        let receipt = block_on_ready(engine.write_batch(&batch))
            .unwrap_or_else(|error| panic!("write should succeed: {error}"));
        let health = block_on_ready(<PostgresRecordingEngine as WriteSink<Row>>::health_check(
            &engine,
        ))
        .unwrap_or_else(|error| panic!("health should succeed: {error}"));

        assert_eq!(affected, 1);
        assert_eq!(
            engine
                .statements()
                .unwrap_or_else(|error| panic!("statements should read: {error}")),
            vec!["insert into bars"]
        );
        assert_eq!(rows[0]["engine"], "postgres-recording");
        assert_eq!(receipt.rows_written, 1);
        assert_eq!(
            engine
                .rows_written()
                .unwrap_or_else(|error| panic!("row count should read: {error}")),
            1
        );
        assert_eq!(health, HealthStatus::Healthy);
        assert!(block_on_ready(engine.execute("", serde_json::json!({}))).is_err());
    }

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future should be ready without an executor"),
        }
    }
}
