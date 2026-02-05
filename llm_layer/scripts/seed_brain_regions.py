#!/usr/bin/env python3
"""
Seed brain_region_responses table with sample data for demo.
Run from llm_layer directory: python scripts/seed_brain_regions.py
"""

import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

from datetime import datetime
from dotenv import load_dotenv

load_dotenv(Path(__file__).parent.parent / ".env")

from db.schema import get_connection

SAMPLE_DATA = [
    {
        "query": "amygdala",
        "region_name": "Amygdala",
        "hemisphere": "Bilateral",
        "lobe": "Temporal",
        "anatomical_region": "Medial temporal lobe, limbic system",
        "function_description": "The amygdala is a key structure for emotional processing, fear conditioning, and threat detection. It integrates sensory inputs and modulates emotional and autonomic responses. It is involved in memory consolidation for emotionally salient events and social cognition.",
        "disease_description": "Anxiety disorders, PTSD, depression, autism spectrum disorder, Kluver-Bucy syndrome, temporal lobe epilepsy, Urbach-Wiethe disease",
    },
    {
        "query": "hippocampus",
        "region_name": "Hippocampus",
        "hemisphere": "Bilateral",
        "lobe": "Temporal",
        "anatomical_region": "Medial temporal lobe",
        "function_description": "The hippocampus is essential for formation of new episodic and declarative memories, spatial navigation, and context encoding. It is part of the limbic system and communicates with cortex and other subcortical structures.",
        "disease_description": "Alzheimer's disease, amnesia, temporal lobe epilepsy, depression, schizophrenia, stress-related memory impairment",
    },
    {
        "query": "paraventricular nucleus hypothalamus",
        "region_name": "Paraventricular Nucleus of the Hypothalamus (PVN)",
        "hemisphere": "Bilateral",
        "lobe": "Diencephalon",
        "anatomical_region": "Hypothalamus - Medial zone, dorsal aspect",
        "function_description": "The paraventricular nucleus (PVN) is a critical neuroendocrine control center within the hypothalamus, serving as the primary source of oxytocin and vasopressin production. It plays a fundamental role in regulating the hypothalamic-pituitary-adrenal (HPA) axis through corticotropin-releasing hormone (CRH) synthesis, which controls stress responses and cortisol release.",
        "disease_description": "Diabetes insipidus, SIADH, adrenal insufficiency, stress-related disorders, PTSD, depression, autism spectrum disorder, Prader-Willi syndrome",
    },
]


def seed():
    conn = get_connection()
    cursor = conn.cursor()
    now = datetime.utcnow()
    inserted = 0
    for row in SAMPLE_DATA:
        try:
            cursor.execute(
                """
                INSERT INTO brain_region_responses (
                    query, query_timestamp, region_name, hemisphere, lobe,
                    anatomical_region, function_description, disease_description,
                    created_at, updated_at
                ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                """,
                (
                    row["query"],
                    now,
                    row["region_name"],
                    row["hemisphere"],
                    row["lobe"],
                    row["anatomical_region"],
                    row["function_description"],
                    row["disease_description"],
                    now,
                    now,
                ),
            )
            inserted += 1
        except Exception as e:
            print(f"Skip {row['region_name']}: {e}")
    conn.commit()
    cursor.close()
    conn.close()
    print(f"✓ Seeded {inserted} brain region(s)")


if __name__ == "__main__":
    seed()
