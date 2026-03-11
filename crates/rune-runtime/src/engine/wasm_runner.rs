//! WASM tool execution — loads .wasm modules and invokes exported `run` function.
//!
//! Tool ABI: the WASM module exports `run` with signature:
//!   (input_ptr: i32, input_len: i32, output_ptr: i32, output_max: i32) -> i32
//! Returns number of bytes written to output, or negative on error.
//! Host writes input JSON to memory at input_ptr, reads output from output_ptr.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rune_spec::ToolDescriptor;
use tokio::sync::RwLock;
use wasmtime::{Engine, Instance, Module, Store};

use crate::error::RuntimeError;

const MAX_OUTPUT_BYTES: usize = 1_048_576; // 1 MB

/// Runner for WASM tools. Caches compiled modules by path.
pub struct WasmToolRunner {
    engine: Engine,
    /// Cache of path -> compiled Module
    module_cache: Arc<RwLock<HashMap<String, Module>>>,
}

impl WasmToolRunner {
    pub fn new() -> anyhow::Result<Self> {
        let config = wasmtime::Config::new();
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            module_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn run(
        &self,
        descriptor: &ToolDescriptor,
        agent_dir: &Path,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        let module_path = agent_dir.join(&descriptor.module);
        if !module_path.exists() {
            return Err(RuntimeError::ToolExecution(format!(
                "WASM tool module not found: {}",
                module_path.display()
            )));
        }

        let path_key = module_path
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| module_path.to_string_lossy().to_string());

        let module = self.get_or_load_module(&path_key, &module_path).await?;

        let input_bytes = serde_json::to_vec(&input)
            .map_err(|e| RuntimeError::ToolExecution(e.to_string()))?;

        if input_bytes.len() > MAX_OUTPUT_BYTES {
            return Err(RuntimeError::ToolExecution(format!(
                "input exceeds {} bytes",
                MAX_OUTPUT_BYTES
            )));
        }

        let module = module.clone();
        let engine = self.engine.clone();
        let result = tokio::time::timeout(
            Duration::from_millis(descriptor.timeout_ms),
            tokio::task::spawn_blocking(move || execute_run_sync(&engine, &module, &input_bytes)),
        )
        .await;

        match result {
            Ok(Ok(inner)) => inner,
            Ok(Err(join_err)) => Err(RuntimeError::ToolExecution(format!(
                "WASM tool task failed: {join_err}"
            ))),
            Err(_elapsed) => Err(RuntimeError::ToolExecution(format!(
                "WASM tool '{}' timed out after {}ms",
                descriptor.name, descriptor.timeout_ms
            ))),
        }
    }

    async fn get_or_load_module(
        &self,
        path_key: &str,
        module_path: &Path,
    ) -> Result<Module, RuntimeError> {
        {
            let cache = self.module_cache.read().await;
            if let Some(m) = cache.get(path_key) {
                return Ok(m.clone());
            }
        }

        let wasm_bytes = std::fs::read(module_path).map_err(|e| {
            RuntimeError::ToolExecution(format!(
                "failed to read WASM module {}: {e}",
                module_path.display()
            ))
        })?;

        let module = Module::new(&self.engine, &wasm_bytes).map_err(|e| {
            RuntimeError::ToolExecution(format!(
                "failed to compile WASM module {}: {e}",
                module_path.display()
            ))
        })?;

        {
            let mut cache = self.module_cache.write().await;
            cache.insert(path_key.to_string(), module.clone());
        }

        Ok(module)
    }

}

fn execute_run_sync(
    engine: &Engine,
    module: &Module,
    input: &[u8],
) -> Result<serde_json::Value, RuntimeError> {
    let mut store = Store::new(engine, ());

    let instance = Instance::new(&mut store, module, &[]).map_err(|e| {
        RuntimeError::ToolExecution(format!("failed to instantiate WASM: {e}"))
    })?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| RuntimeError::ToolExecution("WASM module has no 'memory' export".into()))?;

    let run = instance.get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "run").map_err(|_| {
        RuntimeError::ToolExecution(
            "WASM tool must export 'run(input_ptr, input_len, output_ptr, output_max) -> output_len'".into(),
        )
    })?;

    let page_size = 65536;
    let input_len = input.len() as i32;
    let output_max = MAX_OUTPUT_BYTES as i32;

    let mem_size = memory.data_size(&store);
    let required = (input_len as usize)
        .saturating_add(output_max as usize)
        .max(64);
    if mem_size < required {
        let pages_needed = (required + page_size - 1) / page_size;
        memory.grow(&mut store, pages_needed as u64).map_err(|e| {
            RuntimeError::ToolExecution(format!("WASM memory grow failed: {e}"))
        })?;
    }

    {
        let mem_bytes = memory.data_mut(&mut store);
        mem_bytes[..input.len()].copy_from_slice(input);
    }

    let out_len = run.call(&mut store, (0, input_len, input_len, output_max)).map_err(|e| {
        RuntimeError::ToolExecution(format!("WASM run() failed: {e}"))
    })?;

    if out_len < 0 {
        let err_slice = {
            let mem_bytes = memory.data(&store);
            let start = input.len();
            let end = (start + 256).min(mem_bytes.len());
            mem_bytes[start..end].to_vec()
        };
        let stderr_hint = String::from_utf8_lossy(&err_slice).to_string();
        return Err(RuntimeError::ToolExecution(format!(
            "WASM tool returned error (code {out_len}){}",
            if stderr_hint.trim().is_empty() {
                String::new()
            } else {
                format!(": {stderr_hint}")
            }
        )));
    }

    let out_len = out_len as usize;
    if out_len > MAX_OUTPUT_BYTES {
        return Err(RuntimeError::ToolExecution(format!(
            "WASM tool output exceeds {} bytes",
            MAX_OUTPUT_BYTES
        )));
    }

    let mem_bytes = memory.data(&store);
    let output_slice = &mem_bytes[input.len()..input.len() + out_len];
    let output = serde_json::from_slice(output_slice).map_err(|e| {
        let raw = String::from_utf8_lossy(output_slice);
        RuntimeError::ToolExecution(format!("WASM tool returned invalid JSON: {e}\nraw: {raw}"))
    })?;

    Ok(output)
}
