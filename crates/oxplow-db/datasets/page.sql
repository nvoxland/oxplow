SELECT uri, extension, id, title, description, label, icon, category, keywords,
       params_json AS params, renders_json AS renders, is_default AS "default", pinned, hidden, tool,
       needs_json AS needs, source, deployment, running_uri
FROM source('page')
