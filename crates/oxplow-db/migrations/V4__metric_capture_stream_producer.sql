-- The tree-capture prune and the baseline reads find one producer's
-- captures in one stream: with the stream's index alone, each of ~45
-- producers scanned all of the stream's ~100k captures.
CREATE INDEX idx_metric_capture_stream_producer ON metric_capture(stream_id, producer, captured_at);
