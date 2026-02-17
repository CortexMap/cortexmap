### Orch

- [x] Summary generation flow is incorrect. We are just chunking files and adding them as user queries, instad, we
  should insert the chunks in vetor DB, and let LLM make tool call, we execute the query and pass the result.

### Fetcher

- [ ] In progress count is incorrect.
- [x] Query is incorrect, it must be in form of Boolean query.

### Brainatlas

- [x] Modify system prompts to show better results, currently, it's summarizing the chunks, instead, it should use the
  chunks
  and generate a summary, potential diseases in the region, regions affected with that diseases, symptoms, etc.
