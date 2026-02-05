"""
Quick test to verify database storage without calling the LLM.
Creates a mock BrainRegion object and stores it.
"""

import sys
from pathlib import Path
from dotenv import load_dotenv

sys.path.append(str(Path(__file__).parent.parent))

from query.queryllm import BrainRegion, Location, FunctionDiseases, Hemisphere
from db.repository import store_brain_region_response, get_recent_responses, get_statistics

def test_database_storage():
    """Test storing a mock brain region response"""
    
    # Create a mock BrainRegion object
    mock_region = BrainRegion(
        name="Hippocampus",
        location=Location(
            hemisphere=Hemisphere.BILATERAL,
            lobe="Temporal Lobe",
            anatomical_region="Medial temporal lobe, beneath the cortical surface"
        ),
        function_diseases=FunctionDiseases(
            function_description="The hippocampus plays a crucial role in the formation of new memories and spatial navigation. It is essential for consolidating information from short-term memory to long-term memory and is involved in recalling past experiences. The hippocampus also contributes to spatial memory, helping individuals navigate their environment and remember locations. Additionally, it plays a role in emotional regulation and is connected to other brain regions involved in memory processing. Damage to the hippocampus can result in anterograde amnesia, where new memories cannot be formed, though older memories may remain intact. The hippocampus is one of the first regions affected in Alzheimer's disease, leading to memory impairment. This region is also involved in pattern separation and completion, allowing us to distinguish similar experiences and recall complete memories from partial cues. Studies have shown that the hippocampus continues to generate new neurons throughout life through adult neurogenesis. The hippocampus consists of several subfields including CA1, CA2, CA3, and the dentate gyrus, each contributing differently to memory processing. It receives input from the entorhinal cortex and projects to various cortical and subcortical regions. The left hippocampus is more involved in verbal memory while the right hippocampus processes visual and spatial memories. Research has demonstrated that the hippocampus is critical for episodic memory formation and retrieval.",
            disease_description="The hippocampus is particularly vulnerable to several neurological conditions. Alzheimer's disease is characterized by significant hippocampal atrophy, leading to progressive memory loss and cognitive decline. Temporal lobe epilepsy often involves the hippocampus, with seizures originating in this region causing memory disturbances and potential structural damage over time. Hypoxia or ischemia can result in selective hippocampal damage due to its high metabolic demands. Depression and chronic stress have been associated with reduced hippocampal volume and impaired neurogenesis. Post-traumatic stress disorder is also linked to hippocampal dysfunction, affecting memory consolidation and recall of traumatic events. Additionally, hippocampal sclerosis is a common finding in patients with chronic epilepsy and is characterized by neuronal loss and gliosis in this critical brain structure. Other conditions affecting the hippocampus include encephalitis, particularly herpes simplex encephalitis which has a predilection for the medial temporal lobes. Traumatic brain injury can also damage the hippocampus leading to memory deficits. Furthermore, chronic alcohol abuse has been shown to cause hippocampal shrinkage and impaired memory function. Neurodegenerative disorders beyond Alzheimer's such as frontotemporal dementia may also involve hippocampal pathology."
        )
    )
    
    print("Testing database storage...")
    print("-" * 80)
    
    # Store the mock region
    record_id = store_brain_region_response(
        query="Tell me about the hippocampus",
        brain_region=mock_region,
    )
    
    if record_id:
        print(f"\n✓ Successfully stored mock response with ID: {record_id}")
        
        # Retrieve and display
        print("\nRecent responses in database:")
        recent = get_recent_responses(limit=5)
        for response in recent:
            print(f"  - ID: {response['id']}, Region: {response['region_name']}, Query: {response['query']}")
        
        # Show statistics
        stats = get_statistics()
        print(f"\nDatabase Statistics:")
        print(f"  Total responses: {stats['total_responses']}")
        print(f"  Unique regions: {stats['unique_regions']}")
        print(f"  Latest query: {stats['latest_query']}")
    else:
        print("\n✗ Failed to store mock response")
    
    print("-" * 80)

if __name__ == "__main__":
    load_dotenv()
    test_database_storage()
