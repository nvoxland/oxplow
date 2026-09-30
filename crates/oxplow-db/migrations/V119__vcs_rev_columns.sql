-- The file-ref and capture version columns name what they hold: the
-- VCS's own revision id (`Revision::vcs_rev`, a git sha for the git
-- provider), not "a git version" (P5 follow-up tsk542). Values are
-- unchanged. Dashboard tiles pinned with the old names (V108 wrote them)
-- are rewritten; a person's own lens files are theirs to update.
ALTER TABLE metric_capture RENAME COLUMN closest_git_version TO closest_vcs_rev;
ALTER TABLE metric_capture RENAME COLUMN git_version_exact TO vcs_rev_exact;
ALTER TABLE page_ref RENAME COLUMN closest_git_version TO closest_vcs_rev;
ALTER TABLE page_ref RENAME COLUMN git_version_exact TO vcs_rev_exact;
ALTER TABLE effort_file RENAME COLUMN closest_git_version TO closest_vcs_rev;
ALTER TABLE effort_file RENAME COLUMN git_version_exact TO vcs_rev_exact;

UPDATE dashboard_item
SET options_json = replace(
        replace(options_json, 'closest_git_version AS git_version', 'closest_vcs_rev AS vcs_rev'),
        'closest_git_version', 'closest_vcs_rev')
WHERE options_json LIKE '%closest_git_version%';
