SELECT page_uri, position, question, about, params_json AS params
FROM source('page_answer')
