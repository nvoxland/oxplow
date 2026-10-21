SELECT r.target, r.capability, r.bound, r.version, r.props, r.view, r.label
FROM source('replaceable') r
