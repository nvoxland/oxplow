-- Models own the published views (P4.2): they are dropped before the
-- migrations and compiled after them at every open. The views earlier
-- migrations created are dropped here, so no later migration meets one
-- (a fresh database runs every migration in one go, before the first
-- compile).
DROP VIEW IF EXISTS v_agent_nudge;
DROP VIEW IF EXISTS v_agent_turn;
DROP VIEW IF EXISTS v_ai_call;
DROP VIEW IF EXISTS v_branch;
DROP VIEW IF EXISTS v_capture;
DROP VIEW IF EXISTS v_change;
DROP VIEW IF EXISTS v_change_co_change;
DROP VIEW IF EXISTS v_change_duplicate;
DROP VIEW IF EXISTS v_change_file;
DROP VIEW IF EXISTS v_change_function;
DROP VIEW IF EXISTS v_change_import;
DROP VIEW IF EXISTS v_change_test_file;
DROP VIEW IF EXISTS v_claim;
DROP VIEW IF EXISTS v_code_quality_finding;
DROP VIEW IF EXISTS v_code_quality_scan;
DROP VIEW IF EXISTS v_comment;
DROP VIEW IF EXISTS v_commit;
DROP VIEW IF EXISTS v_commit_file;
DROP VIEW IF EXISTS v_commit_task;
DROP VIEW IF EXISTS v_context_read;
DROP VIEW IF EXISTS v_dashboard;
DROP VIEW IF EXISTS v_dashboard_item;
DROP VIEW IF EXISTS v_decision;
DROP VIEW IF EXISTS v_diagnostic;
DROP VIEW IF EXISTS v_effort;
DROP VIEW IF EXISTS v_effort_file;
DROP VIEW IF EXISTS v_effort_metric_delta;
DROP VIEW IF EXISTS v_effort_observation;
DROP VIEW IF EXISTS v_event;
DROP VIEW IF EXISTS v_event_checkpoint;
DROP VIEW IF EXISTS v_event_content;
DROP VIEW IF EXISTS v_event_dead_letter;
DROP VIEW IF EXISTS v_fact;
DROP VIEW IF EXISTS v_measure;
DROP VIEW IF EXISTS v_metric_spec;
DROP VIEW IF EXISTS v_page_visit;
DROP VIEW IF EXISTS v_snapshot;
DROP VIEW IF EXISTS v_snapshot_op;
DROP VIEW IF EXISTS v_stream;
DROP VIEW IF EXISTS v_struggle;
DROP VIEW IF EXISTS v_task;
DROP VIEW IF EXISTS v_task_link;
DROP VIEW IF EXISTS v_task_note;
DROP VIEW IF EXISTS v_test_case;
DROP VIEW IF EXISTS v_test_run;
DROP VIEW IF EXISTS v_thread;
DROP VIEW IF EXISTS v_token_usage;
DROP VIEW IF EXISTS v_tool_call;
DROP VIEW IF EXISTS v_wiki_page;
