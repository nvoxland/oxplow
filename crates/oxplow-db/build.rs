//! Track the migrations dir as a build input. `refinery::embed_migrations!` is
//! a proc macro with no cargo dependency tracking of its own, so without this a
//! MIGRATION-ONLY change (pure SQL, no `.rs` touched — e.g. V64's dim
//! promotion) rebuilds nothing: cargo reuses the cached oxplow-db and every
//! downstream binary silently ships without the new migration. Discovered when
//! V64 didn't apply to a freshly built example.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
