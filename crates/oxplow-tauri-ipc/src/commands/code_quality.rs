//! Tauri adapters for the code-quality command surface. The real
//! bodies live in `oxplow_rpc::commands::code_quality`; each command
//! here is a thin delegate so the headless daemon can dispatch the
//! same cores without `tauri`.

pub use oxplow_rpc::commands::code_quality::{
    AnalyzeFileSpec, AnalyzeFunctionsResult, AnalyzedFileChurn, AnalyzedFileSide, AnalyzedFunction,
    AnalyzedFunctionChurn, FileFilterSpec, ImportDelta,
};
