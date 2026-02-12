from dataclasses import dataclass
from typing import List

import json
from typing import Optional
import re

# Define the expected JSON schema that matches your protobuf
BRAIN_REGION_SCHEMA = {
    "type": "object",
    "properties": {
        "id": {"type": "string", "description": "Unique lowercase identifier"},
        "name": {"type": "string", "description": "Region name"},
        "location": {
            "type": "object",
            "properties": {
                "hemisphere": {
                    "type": "string",
                    "enum": ["Left", "Right", "Bilateral"],
                    "description": "Hemisphere location"
                },
                "lobe": {"type": "string", "description": "Lobe or major region"},
                "anatomical_region": {"type": "string", "description": "Specific anatomical location"}
            },
            "required": ["hemisphere", "lobe", "anatomical_region"]
        },
        "function_diseases": {
            "type": "object",
            "properties": {
                "function_description": {
                    "type": "string",
                    "description": "150-250 word description of functions"
                },
                "disease_description": {
                    "type": "string",
                    "description": "Comma-separated list of associated diseases"
                }
            },
            "required": ["function_description", "disease_description"]
        }
    },
    "required": ["id", "name", "location", "function_diseases"]
}

def create_system_prompt(brain_region_name: str, brain_region_id: str) -> str:
    return f"""You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about the {brain_region_name} and format it as a JSON object matching this exact structure:

{{
  "id": "string (unique lowercase identifier)",
  "name": "string (standard anatomical name)",
  "location": {{
    "hemisphere": "string (Left, Right, or Bilateral)",
    "lobe": "string (Frontal, Parietal, Temporal, Occipital, Diencephalon, Brainstem, Cerebellum, etc.)",
    "anatomical_region": "string (specific anatomical location)"
  }},
  "function_diseases": {{
    "function_description": "string (150-250 words covering primary functions, neurotransmitters, and physiological roles)",
    "disease_description": "string (comma-separated list of associated diseases)"
  }}
}}

REQUIREMENTS:
1. ID: "{brain_region_id}"
2. NAME: "{brain_region_name}"
3. LOCATION: Generate anatomically accurate hemisphere, lobe, and anatomical_region values
4. FUNCTIONS: Write 150-250 words covering:
   - Primary functional roles
   - Neurotransmitters involved
   - Integration with other systems
   - Behavioral/physiological effects
5. DISEASES: List associated conditions as comma-separated values, organized by category (endocrine, metabolic, psychiatric, genetic, traumatic, inflammatory)

CRITICAL: Output ONLY valid JSON that matches the structure above. No additional text, no markdown formatting, no code blocks."""

def validate_and_parse_response(response_text: str) -> Optional[dict]:
    """
    Validate that the LLM response matches the protobuf schema.
    Returns parsed dict if valid, None otherwise.
    """
    try:
        # Extract JSON from response (in case there's extra text)
        json_match = re.search(r'\{.*\}', response_text, re.DOTALL)
        if not json_match:
            print("Error: No JSON found in response")
            return None

        json_str = json_match.group(0)
        data = json.loads(json_str)

        # Validate against schema
        if not _validate_schema(data, BRAIN_REGION_SCHEMA):
            return None

        return data

    except json.JSONDecodeError as e:
        print(f"Error: Invalid JSON in response: {e}")
        return None

def _validate_schema(data: dict, schema: dict) -> bool:
    """Recursively validate data against schema"""
    # Check required fields
    for required_field in schema.get("required", []):
        if required_field not in data:
            print(f"Error: Missing required field '{required_field}'")
            return False

    # Check field types and constraints
    properties = schema.get("properties", {})
    for field_name, field_schema in properties.items():
        if field_name in data:
            value = data[field_name]

            # Check enum constraint
            if "enum" in field_schema:
                if value not in field_schema["enum"]:
                    print(f"Error: Field '{field_name}' value '{value}' not in allowed values: {field_schema['enum']}")
                    return False

            # Check nested object
            if field_schema.get("type") == "object" and isinstance(value, dict):
                if not _validate_schema(value, field_schema):
                    return False

    return True

def convert_to_protobuf_json(data: dict) -> str:
    """Convert validated JSON to protobuf JSON format"""
    return json.dumps(data, indent=2)

# Example usage:
if __name__ == "__main__":
    # Step 1: Create the system prompt
    prompt = create_system_prompt(
        brain_region_name="Paraventricular Nucleus of the Hypothalamus (PVN)",
        brain_region_id="pvn_hypothalamus"
    )

    print("System Prompt:")
    print(prompt)
    print("\n" + "="*80 + "\n")

    # Step 2: In your actual code, call Claude with this prompt
    # response = call_claude_api(system_prompt=prompt, user_message="Generate the brain region data.")

    # Step 3: Parse and validate the response
    # Example response for testing
    example_response = """{
  "id": "pvn_hypothalamus",
  "name": "Paraventricular Nucleus of the Hypothalamus (PVN)",
  "location": {
    "hemisphere": "Bilateral",
    "lobe": "Diencephalon",
    "anatomical_region": "Hypothalamus - Medial zone, dorsal aspect"
  },
  "function_diseases": {
    "function_description": "The paraventricular nucleus (PVN) is a critical neuroendocrine center within the hypothalamus that regulates multiple physiological systems through its projections to the pituitary gland and autonomic nervous system. The PVN synthesizes oxytocin and vasopressin (antidiuretic hormone/ADH), hormones essential for social bonding, water balance, and cardiovascular regulation. As a key component of the hypothalamic-pituitary-adrenal (HPA) axis, the PVN produces corticotropin-releasing hormone (CRH) in response to stress, initiating the cascade that ultimately leads to cortisol release. Beyond neuroendocrine function, the PVN integrates osmotic signals to maintain fluid homeostasis, regulates core body temperature through connections with the dorsomedial hypothalamus and brainstem, and controls energy expenditure and feeding behavior. The nucleus receives convergent input from multiple brain regions monitoring circulating osmolarity, blood pressure, temperature, and emotional state, making it a critical hub for autonomic and endocrine integration.",
    "disease_description": "Diabetes insipidus (central), Syndrome of inappropriate antidiuretic hormone secretion (SIADH), Secondary adrenal insufficiency, Hypogonadism, Secondary hypothyroidism, Hyperthermia, Hypothermia, Anorexia nervosa, Bulimia nervosa, Obesity and metabolic syndrome, Anxiety disorders, Post-traumatic stress disorder (PTSD), Depression, Autism spectrum disorder, Prader-Willi syndrome, Kallmann syndrome, Pituitary apoplexy, Hyperprolactinemia, Hypothalamic trauma or injury, Hypothalitis"
  }
}"""

    validated_data = validate_and_parse_response(example_response)

    if validated_data:
        print("✓ Response is valid and matches protobuf schema!")
        print("\nProtobuf JSON format:")
        print(convert_to_protobuf_json(validated_data))
    else:
        print("✗ Response validation failed")
def create_system_prompt(brain_region_name: str, brain_region_id: str) -> str:
    return f"""You are an expert neuroscience knowledge base generator. Your task is to generate accurate, clinically relevant information about the {brain_region_name} and format it as a structured protocol buffer message for a BrainRegion entity.

INSTRUCTIONS:
Generate a complete Brain Region message for the {brain_region_name} with the following requirements:

1. ID FIELD:
   - Use a unique, lowercase identifier: "{brain_region_id}"

2. NAME FIELD:
   - Use the standard anatomical name: "{brain_region_name}"

3. LOCATION MESSAGE (Location):
   - hemisphere: (generate based on anatomical knowledge of {brain_region_name})
   - lobe: (generate based on anatomical knowledge of {brain_region_name})
   - anatomical_region: (generate detailed anatomical location for {brain_region_name})

4. FUNCTION_DISEASES MESSAGE (FunctionAndDiseases):

   4a. function_description: Write a comprehensive but concise description (150-250 words) covering the primary functions, neurotransmitters, and physiological roles of the {brain_region_name}.

   4b. disease_description: List associated neurological, endocrine, metabolic, and psychiatric conditions relevant to {brain_region_name} dysfunction (as a comma-separated list).

QUALITY STANDARDS:
- Ensure all information is anatomically and clinically accurate
- Use evidence-based medical knowledge
- Maintain consistency with modern neuroscience literature
- Be specific and detailed without being verbose
- Include clinically relevant conditions that have established connections to {brain_region_name} dysfunction
- Organize disease descriptions in a logical order (endocrine, metabolic, psychiatric, genetic, etc.)

OUTPUT FORMAT:
Present the complete message in a clear, structured format showing all fields and nested messages properly populated with the generated content."""

# Usage:
prompt = create_system_prompt(
    brain_region_name="Paraventricular Nucleus of the Hypothalamus (PVN)",
    brain_region_id="pvn_hypothalamus"
)