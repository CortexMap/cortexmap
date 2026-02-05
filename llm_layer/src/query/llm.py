from pydantic import BaseModel, Field
from enum import Enum
import ollama
from typing import Literal
import os
from pathlib import Path
from dotenv import load_dotenv

class Hemisphere(str, Enum):
    LEFT = "Left"
    RIGHT = "Right"
    BILATERAL = "Bilateral"

class Location(BaseModel):
    """Location information for a brain region"""
    hemisphere: Hemisphere = Field(
        description="Hemisphere location"
    )
    lobe: str = Field(
        description="Lobe or major region"
    )
    anatomical_region: str = Field(
        description="Specific anatomical location"
    )

class FunctionDiseases(BaseModel):
    """Function and disease information"""
    function_description: str = Field(
        description="150-250 word paragraph description of functions",
        min_length=1500,
        max_length=2000  # Rough character limit for 250 words
    )
    disease_description: str = Field(
        description="150-250 word paragraph description of diseases",
        min_length=1500,
        max_length=2000  # Rough character limit for 250 words
    )

class BrainRegion(BaseModel):
    """Complete brain region information"""
    # id: int = Field(
    #     description="Unique identifier"
    # )
    name: str = Field(
        description="Region name"
    )
    location: Location
    function_diseases: FunctionDiseases

def load_data_from_folder(folder_path: str = os.getenv('MD_DATA_PATH')) -> str:
    """
    Load all markdown files from a folder and combine them into a single string.

    Args:
        folder_path: Path to the folder containing markdown files

    Returns:
        Combined string containing all markdown file contents
    """
    combined_data = ""
    folder = Path(os.getenv('MD_DATA_PATH'))

    if not folder.exists():
        print(f"Warning: Folder '{folder_path}' not found.")
        return combined_data

    # Get all markdown files
    md_files = sorted(folder.glob("*.md"))

    if not md_files:
        print(f"Warning: No markdown files found in '{folder_path}'.")
        return combined_data

    # Read and combine all markdown files
    for file_path in md_files:
        try:
            with open(file_path, 'r', encoding='utf-8') as file:
                content = file.read()
                # Add filename as a separator for clarity
                combined_data += f"\n\n--- {file_path.name} ---\n{content}"
        except Exception as e:
            print(f"Error reading file {file_path}: {e}")

    return combined_data

def brain_region_query(region_name: str, include_context: bool = False) -> BrainRegion:
    """Get structured brain region information from LLM"""
    context = ""
    if include_context:
        context = load_data_from_folder()
        if context:
            context = f"\nHere is reference data about brain regions:\n{context}\n\n"

    # Combine context with the query
    prompt = f"You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about {region_name}"
    full_query = f"User query: {prompt}, Context: {context}"

    response = ollama.chat(
        model='deepseek-r1:8b',
        messages=[{
            'role': 'user',
            'content': prompt
        }],
        format=BrainRegion.model_json_schema()  # Use Pydantic schema
    )

    # Parse response into Pydantic model (validates automatically)
    brain_region = BrainRegion.model_validate_json(response.message.content)

    return brain_region

# Example usage
if __name__ == "__main__":
    load_dotenv()
    result = brain_region_query("paraventricular hypothalmus", include_context=True)
    # print(f"ID: {result.id}")
    print(f"Name: {result.name}")
    print(f"Hemisphere: {result.location.hemisphere.value}")
    print(f"Lobe: {result.location.lobe}")
    print(f"Function: {result.function_diseases.function_description}")