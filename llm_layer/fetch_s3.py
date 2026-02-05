import boto3
import os
from dotenv import load_dotenv

load_dotenv()

LOCAL_DIR = os.getenv('MD_DATA_PATH')  # Directory to save files

def download_md_files():

    # Create S3 client
    s3_client = boto3.client(
        's3',
        endpoint_url=os.getenv('ENDPOINT_URL'),
        aws_access_key_id=os.getenv('ACCESS_KEY'),
        aws_secret_access_key=os.getenv('SECRET_KEY'),
        region_name='us-east-1'
    )

    try:
        # List all objects in bucket
        paginator = s3_client.get_paginator('list_objects_v2')
        pages = paginator.paginate(Bucket=os.getenv('BUCKET_NAME'))

        downloaded_count = 0

        for page in pages:
            if 'Contents' not in page:
                continue

            for obj in page['Contents']:
                key = obj['Key']

                # Check if file ends with .md
                if key.endswith('.md'):
                    # Replace path separators with underscores to create unique filename
                    unique_filename = key.replace('/', '_')
                    local_path = os.path.join(LOCAL_DIR, unique_filename)

                    # Ensure directory exists
                    os.makedirs(LOCAL_DIR, exist_ok=True)

                    print(f"Downloading {key} -> {unique_filename}...")
                    s3_client.download_file(os.getenv('BUCKET_NAME'), key, local_path)
                    downloaded_count += 1

        print(f"\n✓ Successfully downloaded {downloaded_count} .md files to '{LOCAL_DIR}'")

    except Exception as e:
        print(f"✗ Error: {e}")

if __name__ == '__main__':
    download_md_files()