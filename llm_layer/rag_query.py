import argparse
from typing import List, Tuple

from langchain_ollama import OllamaEmbeddings, ChatOllama
from langchain_community.vectorstores import Chroma
from langchain_core.messages import SystemMessage, HumanMessage
from dotenv import load_dotenv
import os, shutil

load_dotenv()

CHROMA_PATH = os.getenv('CHROMA_DB_PATH')

# Strong, retrieval-grounded system prompt
SYSTEM_PROMPT = """You are a careful, retrieval-grounded assistant.

Rules:
1) Use ONLY the provided context to answer.
2) If the answer is not in the context, say: "I don’t have enough information in the provided context."
3) Be concise, precise, and avoid speculation—no outside knowledge.
4) Include inline citations like [S1], [S2] mapped to the provided sources list.
5) If there are conflicts in the context, note them briefly and state the most supported view.

Output format:
- Direct answer in 2–6 sentences.
- Short bullet points allowed for lists.
- Include inline source tags [S#] where each # maps to the source order provided below.
"""

def format_context(docs_with_scores: List[Tuple]):
    # Join top-k chunks and build source map
    context_parts = []
    sources = []
    for i, (doc, score) in enumerate(docs_with_scores, start=1):
        src = doc.metadata.get("source", f"doc_{i}")
        sources.append(src)
        # Keep each chunk small-ish; model context is limited
        context_parts.append(f"[S{i}] {doc.page_content.strip()}")
    context_text = "\n\n---\n\n".join(context_parts)
    return context_text, sources

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("query_text", type=str, help="The query text.")
    parser.add_argument("--k", type=int, default=4, help="Top-k chunks to retrieve.")
    parser.add_argument("--min_relevance", type=float, default=0.65, help="Min relevance threshold.")
    parser.add_argument("--embed_model", type=str, default="nomic-embed-text",
                        help="Ollama embeddings model name (e.g., nomic-embed-text, bge-m3).")
    parser.add_argument("--chat_model", type=str, default="llama3.1:8b",
                        help="Ollama chat model name (e.g., llama3.1:8b, qwen2.5:7b).")
    args = parser.parse_args()

    # Embeddings + Vector DB (persisted Chroma)
    embeddings = OllamaEmbeddings(model=args.embed_model)
    db = Chroma(persist_directory=CHROMA_PATH, embedding_function=embeddings)

    # Retrieve
    results = db.similarity_search_with_relevance_scores(args.query_text, k=args.k)
    if not results:
        print("Unable to find matching results.")
        return
    # Filter by threshold
    results = [(d, s) for (d, s) in results if s >= args.min_relevance]
    if not results:
        print("No results above relevance threshold.")
        return

    # Build context + source list
    context_text, sources = format_context(results)

    # Build messages
    messages = [
        SystemMessage(content=SYSTEM_PROMPT),
        HumanMessage(content=(
            f"Context:\n{context_text}\n\n"
            f"---\n\nQuestion: {args.query_text}\n\n"
            f"Sources (in order): {sources}"
        )),
    ]

    # Local chat model (Ollama)
    llm = ChatOllama(model=args.chat_model, temperature=0.0)

    # Run
    response = llm.invoke(messages)
    print("\n" + "-" * 80)
    print(response.content.strip())
    print("\nSources:", sources)

if __name__ == "__main__":
    main()
