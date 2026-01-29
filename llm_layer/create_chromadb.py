from langchain_ollama import OllamaEmbeddings
from langchain_text_splitters import RecursiveCharacterTextSplitter
from langchain_community.vectorstores import Chroma
from langchain_core.documents import Document
from dotenv import load_dotenv
import os, shutil

load_dotenv()

CHROMA_PATH = "chroma"
DATA_PATH = "./papers_md"

def main():
    generate_data_store()

def generate_data_store():
    documents = load_documents()
    chunks = split_text(documents)
    save_to_chroma(chunks)

def load_documents():
    from langchain_community.document_loaders import DirectoryLoader
    loader = DirectoryLoader(DATA_PATH, glob="*.md")
    return loader.load()

def split_text(documents: list[Document]):
    text_splitter = RecursiveCharacterTextSplitter(
        chunk_size=300, chunk_overlap=100, length_function=len, add_start_index=True
    )
    chunks = text_splitter.split_documents(documents)
    print(f"Split {len(documents)} documents into {len(chunks)} chunks.")
    print(chunks[10].page_content)
    print(chunks[10].metadata)
    return chunks

def save_to_chroma(chunks: list[Document]):
    # Clear out local Chroma DB
    if os.path.exists(CHROMA_PATH):
        shutil.rmtree(CHROMA_PATH)

    embeddings = OllamaEmbeddings(model="nomic-embed-text")

    db = Chroma.from_documents(
        documents=chunks,
        embedding=embeddings,
        persist_directory=CHROMA_PATH,
    )
    db.persist()
    print(f"Saved {len(chunks)} chunks to {CHROMA_PATH}.")

if __name__ == "__main__":
    main()
