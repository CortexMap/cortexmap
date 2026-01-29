import argparse
from typing import List, Tuple
from datetime import datetime
import sys
import signal

from langchain_ollama import OllamaEmbeddings, ChatOllama
from langchain_chroma import Chroma  # Updated import
from langchain_core.messages import SystemMessage, HumanMessage
from dotenv import load_dotenv
import os

load_dotenv()

CHROMA_PATH = os.getenv('CHROMA_DB_PATH')

# Strong, retrieval-grounded system prompt
SYSTEM_PROMPT = """You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about the paraventricular hypothalamus and format it as a structured protocol buffer message for a BrainRegion entity.

                   INSTRUCTIONS:
                   Generate a complete BrainRegion message for the paraventricular hypothalamus with the following requirements:

                   1. ID FIELD:
                      - Use a unique, lowercase identifier: "pvn_hypothalamus"

                   2. NAME FIELD:
                      - Use the standard anatomical name: "Paraventricular Nucleus of the Hypothalamus (PVN)"

                   3. LOCATION MESSAGE (Location):
                      - hemisphere: "Bilateral" (this structure exists in both hemispheres)
                      - lobe: "Diencephalon" (the hypothalamus is part of the diencephalon, not a cerebral lobe)
                      - anatomical_region: "Hypothalamus - Medial zone, dorsal aspect"

                   4. FUNCTION_DISEASES MESSAGE (FunctionAndDiseases):

                      4a. function_description: Write a comprehensive but concise description (150-250 words) that covers:
                          - Primary role in neuroendocrine function and the hypothalamic-pituitary axis
                          - Neurotransmitter synthesis (oxytocin and vasopressin/ADH production)
                          - Roles in osmotic and cardiovascular regulation
                          - Stress response and corticotropin-releasing hormone (CRH) production
                          - Thermoregulation and energy homeostasis
                          - Its position as a key integration center for autonomic and endocrine functions
                          - Circadian rhythm regulation connections
                          - Social behavior and bonding (oxytocin-related)

                      4b. disease_description: List associated neurological, endocrine, and psychiatric conditions (as a comma-separated list):
                          - Diabetes insipidus (central/neurogenic)
                          - Syndrome of inappropriate antidiuretic hormone secretion (SIADH)
                          - Adrenal insufficiency (secondary to PVN dysfunction)
                          - Hypogonadism
                          - Hypothyroidism (secondary)
                          - Hyperthermia/Hypothermia (thermoregulation disorders)
                          - Anorexia nervosa
                          - Bulimia nervosa
                          - Obesity and metabolic syndrome
                          - Stress-related disorders and anxiety
                          - Post-traumatic stress disorder (PTSD)
                          - Depression (related to HPA axis dysfunction)
                          - Autism spectrum disorder (oxytocin system involvement)
                          - Prader-Willi syndrome
                          - Kallmann syndrome
                          - Pituitary apoplexy (affecting PVN inputs)
                          - Hyperprolactinemia
                          - Trauma or injury to the hypothalamus
                          - Inflammatory conditions (hypothalitis)

                   QUALITY STANDARDS:
                   - Ensure all information is anatomically and clinically accurate
                   - Use evidence-based medical knowledge
                   - Maintain consistency with modern neuroscience literature
                   - Be specific and detailed without being verbose
                   - Include clinically relevant conditions that have established connections to PVN dysfunction
                   - Organize disease descriptions in a logical order (endocrine, metabolic, psychiatric, genetic)

                   OUTPUT FORMAT:
                   Present the complete message in a clear, structured format showing all fields and nested messages properly populated with the generated content.
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

def generate_markdown(response_content: str, sources: List[str], query: str) -> str:
    """Generate a markdown document from the response."""
    md = f"""# Brain Region Knowledge Base Entry

Generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}

## Query
{query}

## Generated Content

{response_content}

---

## Sources
{chr(10).join(f'- {src}' for src in sources)}
"""
    return md

def signal_handler(sig, frame):
    """Handle KeyboardInterrupt gracefully."""
    print("\n\n⚠️  Generation interrupted by user.")
    sys.exit(0)

def main():
    # Register signal handler for graceful interruption
    signal.signal(signal.SIGINT, signal_handler)

    parser = argparse.ArgumentParser()
    parser.add_argument("query_text", type=str, help="The query text.")
    parser.add_argument("--k", type=int, default=4, help="Top-k chunks to retrieve.")
    parser.add_argument("--min_relevance", type=float, default=0.40, help="Min relevance threshold.")
    parser.add_argument("--embed_model", type=str, default="nomic-embed-text:latest",
                        help="Ollama embeddings model name (e.g., nomic-embed-text, bge-m3).")
    parser.add_argument("--chat_model", type=str, default="gemma3:latest",
                        help="Ollama chat model name (e.g., llama3.1:8b, qwen2.5:7b).")
    parser.add_argument("--output", type=str, default=None,
                        help="Optional output file path to save markdown (if not provided, only prints to stdout).")
    parser.add_argument("--timeout", type=int, default=300,
                        help="Timeout in seconds for LLM response (default: 300s = 5 minutes).")
    args = parser.parse_args()

    try:
        print(f"📚 Loading embeddings model: {args.embed_model}")
        embeddings = OllamaEmbeddings(model=args.embed_model)

        print(f"🔍 Loading Chroma database from: {CHROMA_PATH}")
        db = Chroma(persist_directory=CHROMA_PATH, embedding_function=embeddings)

        print(f"🔎 Retrieving top-{args.k} documents with min relevance {args.min_relevance}...")
        # Retrieve
        results = db.similarity_search_with_relevance_scores(args.query_text, k=args.k)
        if not results:
            print("❌ Unable to find matching results.")
            return

        # Filter by threshold
        results = [(d, s) for (d, s) in results if s >= args.min_relevance]
        if not results:
            print(f"❌ No results above relevance threshold ({args.min_relevance}).")
            return

        print(f"✓ Found {len(results)} relevant document(s)")

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
        print(f"🤖 Initializing chat model: {args.chat_model}")
        llm = ChatOllama(model=args.chat_model, temperature=0.0, timeout=args.timeout)

        print(f"⏳ Generating response (timeout: {args.timeout}s)... Press Ctrl+C to cancel")
        # Run
        response = llm.invoke(messages)
        response_content = response.content.strip()

        # Generate markdown
        markdown_output = generate_markdown(response_content, sources, args.query_text)

        # Print to stdout
        print("\n" + "=" * 80)
        print(markdown_output)
        print("=" * 80)

        # Optionally save to file
        if args.output:
            with open(args.output, 'w') as f:
                f.write(markdown_output)
            print(f"\n✓ Markdown output saved to: {args.output}")

    except Exception as e:
        print(f"\n❌ Error: {type(e).__name__}: {str(e)}")
        sys.exit(1)

if __name__ == "__main__":
    main()