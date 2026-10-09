//! `oxplow-script-worker`: runs one Starlark or jaq script for its host
//! and exits (`oxplow_script::worker`). Its allocator counts live bytes,
//! so a run past its memory cap ends the process instead of the machine.

#[global_allocator]
static ALLOCATOR: oxplow_script::worker::MemoryCap = oxplow_script::worker::MemoryCap;

fn main() {
    oxplow_script::worker::main()
}
