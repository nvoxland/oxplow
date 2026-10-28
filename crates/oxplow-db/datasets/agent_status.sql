-- A thread's sessions rolled up by oxplow_domain::agent::roll_up_status's
-- rule: awaiting_user > stalled > running > error > stopped > idle.
SELECT thread_id, state, detail, since
FROM (
  SELECT st.thread_id, st.state, st.detail, st.since,
         row_number() OVER (
           PARTITION BY st.thread_id
           ORDER BY CASE st.state
                      WHEN 'awaiting_user' THEN 5
                      WHEN 'stalled' THEN 4
                      WHEN 'running' THEN 3
                      WHEN 'error' THEN 2
                      WHEN 'stopped' THEN 1
                      ELSE 0
                    END DESC,
                    st.since DESC
         ) AS rank
  FROM ref('agent_session_status') st
)
WHERE rank = 1
