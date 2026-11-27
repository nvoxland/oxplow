//! `refinery::embed_migrations!` is a proc macro with no dependency
//! tracking of its own: without this, a change to a migration alone
//! rebuilds nothing and the host ships without it.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
