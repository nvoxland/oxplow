/// Foundation's fallback renderer (`oxplow:resource`): what a URI no page
/// renders opens on. It says what the URI names — its resource's label, the
/// title its dataset gives it, its params — or, when nothing declares its
/// resource or its dataset has no such row, shows the URI as a dead link:
/// muted, as written, never an error (`.context/resources.md` "Using URIs").
import { Page } from "../tabs/Page.js";
import { useExtensionRefTitle, useResources } from "../resources.js";
import { identityOf, parseUri, typeOf } from "../uri/uri.js";

export function ResourcePage({ uri }: { uri: string }) {
  const resources = useResources();
  const resource = resources.get(typeOf(uri) ?? "") ?? null;
  const title = useExtensionRefTitle(uri);
  const params = parseUri(uri)?.params ?? {};
  // Dead: nothing declares it, or the dataset that names its rows has none.
  const dead = resource === null || (resource.resolve !== null && title === null);
  return (
    <Page testId="page-resource" title={title ?? resource?.label ?? uri} kind={resource?.label ?? "Link"}>
      <div style={{ padding: 16 }}>
        {dead ? (
          <p data-testid="resource-dead-link" style={{ color: "var(--text-secondary)" }}>
            <code>{identityOf(uri) ?? uri}</code>
            <br />
            {resource === null ? "No enabled extension declares what this link names." : `No ${resource.label.toLowerCase()} by this link exists any more.`}
          </p>
        ) : (
          <dl data-testid="resource-params">
            {Object.entries(params).map(([k, v]) => (
              <div key={k} style={{ display: "flex", gap: 8 }}>
                <dt style={{ color: "var(--text-secondary)" }}>{k}</dt>
                <dd style={{ margin: 0 }}>
                  <code>{v}</code>
                </dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </Page>
  );
}
