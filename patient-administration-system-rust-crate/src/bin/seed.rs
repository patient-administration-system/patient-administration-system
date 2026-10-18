//! `pas-seed` — **stubbed in v0.44.0 Phase 2**.
//!
//! The v0.43.0 seed bin populated `patients` / `practitioners` rows
//! directly. Those tables are gone in v0.44.0 Option C. Phase 3
//! restores a version of this bin that provisions person + worker
//! rows in the upstream services via `PersonClient::create` /
//! `WorkerClient::create`, then writes the dependent PAS rows
//! (facility / ward / room / bed / letter template) referencing the
//! returned UUIDs.

fn main() {
    eprintln!(
        "pas-seed is stubbed during v0.44.0 Phase 2. Phase 3 reintroduces \
         a version that provisions person + worker rows in person-service \
         and worker-service before writing the PAS-owned dependent rows."
    );
    std::process::exit(0);
}
