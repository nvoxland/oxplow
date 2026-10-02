-- P7.B2: a model's freshness policy. NULL is the default (a view,
-- computed on read); `on_change` is stored in its table (`m_<view>`) and
-- recomputed, whole, when one of its inputs changes.
ALTER TABLE model ADD COLUMN materialize TEXT CHECK (materialize IN ('on_change'));
