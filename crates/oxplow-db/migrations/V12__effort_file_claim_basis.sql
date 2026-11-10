-- How a claimed file was claimed: `named` by an edit tool's own input, or
-- `reported` by the harness's statement that a shell call changed it.
-- NULL for an observed file. A value Rust owns, so no CHECK.
ALTER TABLE effort_file ADD COLUMN claim_basis TEXT;
UPDATE effort_file SET claim_basis = 'named' WHERE source = 'claimed';
