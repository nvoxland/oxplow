-- oxplow's code metrics are oxplow-foundation's collectors and metrics, no
-- longer core's `built-in` catalog. Their specs become the extension's
-- rows (stored as `global` + `extension`, like any extension's), and their
-- captures, runs and health carry the new owner, so history and a
-- failure-disabled collector carry over.
UPDATE metric_spec
   SET scope = 'global', extension = 'oxplow-foundation'
 WHERE scope = 'built-in'
   AND key IN ('oxplow.todos', 'oxplow.fn_count', 'oxplow.high_complexity_fns',
               'oxplow.long_functions', 'oxplow.doc_coverage', 'oxplow.duplicate_lines',
               'oxplow.rust.unsafe_blocks', 'oxplow.rust.unwrap_expect_calls',
               'oxplow.rust.panic_macros', 'oxplow.ts.any_usage',
               'oxplow.ts.non_null_assertions', 'oxplow.ts.console_calls',
               'oxplow.ts.ts_ignore', 'oxplow.clojure.defn_count',
               'oxplow.csharp.empty_catch', 'oxplow.csharp.blocking_async_calls');
UPDATE metric_capture SET scope = 'extension:oxplow-foundation' WHERE scope = 'built-in';
UPDATE collector_run SET owner = 'oxplow-foundation' WHERE owner = 'built-in';
UPDATE contribution_health SET extension = 'oxplow-foundation'
 WHERE extension = 'built-in' AND kind = 'collector';
