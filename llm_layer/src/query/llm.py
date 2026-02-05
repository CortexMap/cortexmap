from pydantic import BaseModel, Field
from enum import Enum
import ollama
from typing import Literal
import os
from pathlib import Path
from dotenv import load_dotenv
from pydantic_ai import Agent
from pydantic_ai.models.openrouter import OpenRouterModel
from pydantic_ai.providers.openrouter import OpenRouterProvider


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

# def brain_region_query(region_name: str, include_context: bool = False) -> BrainRegion:
#     """Get structured brain region information from LLM"""
#     context = ""
#     if include_context:
#         context = load_data_from_folder()
#         if context:
#             context = f"\nHere is reference data about brain regions:\n{context}\n\n"
#
#     # Combine context with the query
#     prompt = f"You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about {region_name}"
#     full_query = f"User query: {prompt}, Context: {context}"
#
#     response = ollama.chat(
#         model='deepseek-r1:8b',
#         messages=[{
#             'role': 'user',
#             'content': prompt
#         }],
#         format=BrainRegion.model_json_schema()  # Use Pydantic schema
#     )
#
#     # Parse response into Pydantic model (validates automatically)
#     brain_region = BrainRegion.model_validate_json(response.message.content)
#
#     return brain_region


def brain_region_query(
        region_name: str,
        include_context: bool = False,
        model_name: str = "gpt-oss-120b:free",
        use_native_output: bool = True
) -> BrainRegion:
    """
    Get structured brain region information from OpenRouter via Pydantic AI

    Args:
        region_name: The brain region to query
        include_context: Whether to include reference data from markdown files
        model_name: The OpenRouter model to use
        use_native_output: Whether to use native structured output (if supported by model)

    Returns:
        BrainRegion: Structured brain region information
    """

    # Load optional context
    context = ""
    if include_context:
        context = load_data_from_folder()
        if context:
            context = f"\n\nReference brain region data:\n{context}"

    # Build the system prompt
    system_prompt = """You are an expert neuroscience knowledge base. 
Your task is to provide accurate, clinically relevant information about brain regions.
Return structured data with precise anatomical information and comprehensive descriptions."""

    # Build the user prompt
    user_prompt = f"""Please provide comprehensive information about the {region_name} brain region.

Include:
1. The hemisphere location (Left, Right, or Bilateral)
2. The lobe or major region it belongs to
3. The specific anatomical location
4. A 150-250 word description of its functions
5. A 150-250 word description of associated diseases and dysfunctions
{context}"""

    # Initialize the OpenRouter model
    api_key = os.getenv('OPENROUTER_API_KEY')
    if not api_key:
        raise ValueError("OPENROUTER_API_KEY environment variable not set")

    provider = OpenRouterProvider(api_key=os.getenv('OPENROUTER_API_KEY'))

    model = OpenRouterModel(
        model_name,
        provider=provider
    )

    # Create an agent with BrainRegion as the output type
    agent = Agent(
        model,
        result_type=BrainRegion,
        system_prompt=system_prompt
    )

    # Run the agent synchronously
    result = agent.run_sync(user_prompt)

    return result.data


def brain_region_query_async(
        region_name: str,
        include_context: bool = False,
        model_name: str = "meta-llama/llama-3.3-70b-instruct"
):
    """
    Async version of brain_region_query for use in async contexts

    Args:
        region_name: The brain region to query
        include_context: Whether to include reference data from markdown files
        model_name: The OpenRouter model to use

    Returns:
        Coroutine that yields BrainRegion
    """
    import asyncio

    # Load optional context
    context = ""
    if include_context:
        context = load_data_from_folder()
        if context:
            context = f"\n\nReference brain region data:\n{context}"

    # Build the system prompt
    system_prompt = """You are an expert neuroscience knowledge base. 
Your task is to provide accurate, clinically relevant information about brain regions.
Return structured data with precise anatomical information and comprehensive descriptions."""

    # Build the user prompt
    user_prompt = f"""Please provide comprehensive information about the {region_name} brain region.

Include:
1. The hemisphere location (Left, Right, or Bilateral)
2. The lobe or major region it belongs to
3. The specific anatomical location
4. A 150-250 word description of its functions
5. A 150-250 word description of associated diseases and dysfunctions
{context}"""

    # Initialize the OpenRouter model
    api_key = os.getenv('OPENROUTER_API_KEY')
    if not api_key:
        raise ValueError("OPENROUTER_API_KEY environment variable not set")

    provider = OpenRouterProvider(api_key=api_key)

    model = OpenRouterModel(
        model_name,
        provider=provider
    )

    # Create an agent with BrainRegion as the output type
    agent = Agent(
        model,
        result_type=BrainRegion,
        system_prompt=system_prompt
    )

    # Return the async coroutine
    return agent.run(user_prompt)


# Example usage
if __name__ == "__main__":
    load_dotenv()
    result = brain_region_query("paraventricular hypothalmus", include_context=True)
    # print(f"ID: {result.id}")
    print(f"Name: {result.name}")
    print(f"Hemisphere: {result.location.hemisphere.value}")
    print(f"Lobe: {result.location.lobe}")
    print(f"Function: {result.function_diseases.function_description}")
