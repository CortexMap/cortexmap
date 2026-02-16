### Orch
- Summary generation flow is incorrect. We are just chunking files and adding them as user queries, instad, we should insert the chunks in vetor DB, and let LLM make tool call, we execute the query and pass the result.

### Fetcher
- In progress count is incorrect.
