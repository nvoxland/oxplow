-- T-E3 (tsk50, epic tsk12): the substrate cutover completes. The V38
-- metric_definition/metric_run/metric_sample/metric_finding cluster has been
-- unread since T-C2/T-C3/T-D/T-E1 and unwritten since T-E2; the fact substrate
-- (measure/dimension/metric_spec/metric_capture/fact, V43+) is the sole store.
-- Children first (findings/samples reference runs + definitions). The V38
-- satellites metric_dimension/metric_subject (superseded by the `dimension`
-- catalog + `subject` registry in V43) go with them.
DROP TABLE IF EXISTS metric_finding;
DROP TABLE IF EXISTS metric_sample;
DROP TABLE IF EXISTS metric_run;
DROP TABLE IF EXISTS metric_dimension;
DROP TABLE IF EXISTS metric_subject;
DROP TABLE IF EXISTS metric_definition;
