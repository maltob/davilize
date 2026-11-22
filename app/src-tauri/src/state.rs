//! Shared application state: one DuckDB engine behind a mutex, plus a handle
//! that can interrupt its running query without waiting for the mutex.

use std::sync::{Arc, Mutex, MutexGuard};

use duckdb::InterruptHandle;
use dv_engine::Engine;

use crate::error::ApiError;

pub struct AppState {
    engine: Mutex<Engine>,
    interrupt: Mutex<Arc<InterruptHandle>>,
}

impl AppState {
    pub fn new() -> Result<Self, ApiError> {
        let engine = Engine::new()?;
        let interrupt = engine.interrupt_handle();
        Ok(Self {
            engine: Mutex::new(engine),
            interrupt: Mutex::new(interrupt),
        })
    }

    /// Locks the engine for one operation. Callers run this on a blocking thread.
    pub fn lock_engine(&self) -> Result<MutexGuard<'_, Engine>, ApiError> {
        self.engine.lock().map_err(|_| {
            ApiError::internal("the query engine is unavailable after an earlier failure")
        })
    }

    /// Replaces the engine (used when a report is opened) and its interrupt handle.
    pub fn replace_engine(&self, next: Engine) -> Result<(), ApiError> {
        let handle = next.interrupt_handle();
        let mut engine = self.lock_engine()?;
        *engine = next;
        *self.interrupt_slot()? = handle;
        Ok(())
    }

    /// Interrupts the running query, if any. Does not wait for the engine lock.
    pub fn cancel(&self) -> Result<(), ApiError> {
        self.interrupt_slot()?.interrupt();
        Ok(())
    }

    fn interrupt_slot(&self) -> Result<MutexGuard<'_, Arc<InterruptHandle>>, ApiError> {
        self.interrupt.lock().map_err(|_| {
            ApiError::internal("the query engine is unavailable after an earlier failure")
        })
    }
}
