from pydantic import BaseModel, Field
from enum import Enum
import ollama
from typing import Literal


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

def brain_region_query(query: str) -> BrainRegion:
    """Get structured brain region information from LLM"""

    response = ollama.chat(
        model='deepseek-r1:8b',
        messages=[{
            'role': 'user',
            'content': query
        }],
        format=BrainRegion.model_json_schema()  # Use Pydantic schema
    )

    # Parse response into Pydantic model (validates automatically)
    brain_region = BrainRegion.model_validate_json(response.message.content)

    return brain_region


# Example usage
if __name__ == "__main__":
    result = brain_region_query("Tell me about the hippocampus.")

    # print(f"ID: {result.id}")
    print(f"Name: {result.name}")
    print(f"Hemisphere: {result.location.hemisphere.value}")
    print(f"Lobe: {result.location.lobe}")
    print(f"Function: {result.function_diseases.function_description}")