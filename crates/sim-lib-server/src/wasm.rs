use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

use sim_kernel::{Error, Result};
use sim_wasm_abi::{Handle, WasmFrameLimits, WasmRuntime, WasmiRuntime};

#[derive(Clone)]
pub(crate) struct WasmRegion {
    pub(crate) runtime: Arc<dyn WasmRuntime>,
    pub(crate) module: Handle,
}

fn wasm_regions() -> &'static Mutex<BTreeMap<String, Arc<WasmRegion>>> {
    static REGISTRY: OnceLock<Mutex<BTreeMap<String, Arc<WasmRegion>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Registers a wasm guest module under `region` using the default runtime.
///
/// Instantiates `wasm_bytes` with a wasmi runtime at default frame limits and
/// stores it in the process-wide region registry.
pub fn register_wasm_region(region: &str, wasm_bytes: &[u8]) -> Result<()> {
    let runtime: Arc<dyn WasmRuntime> =
        Arc::new(WasmiRuntime::with_limits(WasmFrameLimits::default()));
    register_wasm_region_with_runtime(region, runtime, wasm_bytes)
}

pub(crate) fn register_wasm_region_with_runtime(
    region: &str,
    runtime: Arc<dyn WasmRuntime>,
    wasm_bytes: &[u8],
) -> Result<()> {
    let module = runtime.instantiate_bytes(wasm_bytes)?;
    let mut registry = wasm_regions()
        .lock()
        .map_err(|_| Error::HostError("wasm region registry mutex poisoned".to_owned()))?;
    registry.insert(region.to_owned(), Arc::new(WasmRegion { runtime, module }));
    Ok(())
}

pub(crate) fn lookup_wasm_region(region: &str) -> Result<Arc<WasmRegion>> {
    let registry = wasm_regions()
        .lock()
        .map_err(|_| Error::HostError("wasm region registry mutex poisoned".to_owned()))?;
    registry
        .get(region)
        .cloned()
        .ok_or_else(|| Error::Eval(format!("unknown wasm region {region}")))
}

#[cfg(test)]
mod tests {
    use super::{lookup_wasm_region, register_wasm_region};
    use std::sync::atomic::{AtomicU64, Ordering};

    use sim_kernel::Error;
    use wat::parse_str as wat_parse_str;

    static NEXT_REGION_ID: AtomicU64 = AtomicU64::new(1);

    fn unique_region_name() -> String {
        format!(
            "test-r6-region-{}",
            NEXT_REGION_ID.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn minimal_wasm_guest_bytes() -> Vec<u8> {
        wat_parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "sim_alloc") (param i32) (result i32) i32.const 0)
                (func (export "sim_manifest") (result i64) i64.const 0)
                (func (export "sim_exports") (result i64) i64.const 0)
                (func (export "sim_call") (param i32 i32 i32 i32) (result i64) i64.const 0)
            )"#,
        )
        .expect("hand-written wasm guest fixture should assemble")
    }

    #[test]
    fn register_wasm_region_instantiates_a_wasmi_module() {
        let region = unique_region_name();
        let bytes = minimal_wasm_guest_bytes();

        register_wasm_region(&region, &bytes).unwrap();

        let registered = lookup_wasm_region(&region).unwrap();
        assert_eq!(registered.module.0, 1);
    }

    #[test]
    fn lookup_wasm_region_reports_unknown_region_clearly() {
        let err = match lookup_wasm_region("missing-r6-region") {
            Ok(_) => panic!("expected unknown wasm region lookup to fail"),
            Err(err) => err,
        };
        assert!(
            matches!(err, Error::Eval(message) if message == "unknown wasm region missing-r6-region")
        );
    }
}
