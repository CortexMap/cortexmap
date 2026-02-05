"""
S3 Downloader Module

Provides modular functionality for downloading markdown files from S3-compatible storage.
"""

import os
from pathlib import Path
from typing import Optional, List, Tuple
import boto3
from botocore.exceptions import ClientError
from dotenv import load_dotenv


class S3Config:
    """Configuration manager for S3 connection parameters."""
    
    def __init__(self):
        """Initialize S3 configuration from environment variables."""
        load_dotenv()
        self.endpoint_url = os.getenv('ENDPOINT_URL')
        self.access_key = os.getenv('ACCESS_KEY')
        self.secret_key = os.getenv('SECRET_KEY')
        self.bucket_name = os.getenv('BUCKET_NAME')
        self.local_dir = os.getenv('MD_DATA_PATH')
        self.region_name = 'us-east-1'
    
    def validate(self) -> Tuple[bool, Optional[str]]:
        """
        Validate that all required configuration parameters are present.
        
        Returns:
            Tuple of (is_valid, error_message)
        """
        required_params = {
            'ENDPOINT_URL': self.endpoint_url,
            'ACCESS_KEY': self.access_key,
            'SECRET_KEY': self.secret_key,
            'BUCKET_NAME': self.bucket_name,
            'MD_DATA_PATH': self.local_dir
        }
        
        missing = [key for key, value in required_params.items() if not value]
        
        if missing:
            return False, f"Missing required environment variables: {', '.join(missing)}"
        
        return True, None


class S3Client:
    """Wrapper for boto3 S3 client operations."""
    
    def __init__(self, config: S3Config):
        """
        Initialize S3 client with configuration.
        
        Args:
            config: S3Config instance with connection parameters
        """
        self.config = config
        self.client = self._create_client()
    
    def _create_client(self):
        """Create and return a configured boto3 S3 client."""
        return boto3.client(
            's3',
            endpoint_url=self.config.endpoint_url,
            aws_access_key_id=self.config.access_key,
            aws_secret_access_key=self.config.secret_key,
            region_name=self.config.region_name
        )
    
    def list_objects(self, prefix: str = '', suffix: str = '') -> List[str]:
        """
        List objects in the S3 bucket with optional filtering.
        
        Args:
            prefix: Filter objects by prefix
            suffix: Filter objects by suffix (e.g., '.md')
        
        Returns:
            List of object keys
        """
        try:
            paginator = self.client.get_paginator('list_objects_v2')
            pages = paginator.paginate(Bucket=self.config.bucket_name)
            
            keys = []
            for page in pages:
                if 'Contents' not in page:
                    continue
                
                for obj in page['Contents']:
                    key = obj['Key']
                    if key.startswith(prefix) and key.endswith(suffix):
                        keys.append(key)
            
            return keys
        
        except ClientError as e:
            raise Exception(f"Failed to list objects: {e}")
    
    def download_file(self, key: str, local_path: str) -> None:
        """
        Download a file from S3 to local path.
        
        Args:
            key: S3 object key
            local_path: Local file path to save to
        """
        try:
            self.client.download_file(self.config.bucket_name, key, local_path)
        except ClientError as e:
            raise Exception(f"Failed to download {key}: {e}")


class FileManager:
    """Manages local file operations for downloaded content."""
    
    @staticmethod
    def ensure_directory(directory: str) -> None:
        """
        Ensure a directory exists, creating it if necessary.
        
        Args:
            directory: Path to directory
        """
        os.makedirs(directory, exist_ok=True)
    
    @staticmethod
    def sanitize_filename(s3_key: str) -> str:
        """
        Convert S3 key to a safe local filename.
        
        Args:
            s3_key: S3 object key
        
        Returns:
            Sanitized filename
        """
        return s3_key.replace('/', '_')
    
    @staticmethod
    def get_local_path(local_dir: str, s3_key: str) -> str:
        """
        Generate local file path for an S3 key.
        
        Args:
            local_dir: Base directory for downloads
            s3_key: S3 object key
        
        Returns:
            Full local file path
        """
        filename = FileManager.sanitize_filename(s3_key)
        return os.path.join(local_dir, filename)


class S3Downloader:
    """Main class for downloading files from S3."""
    
    def __init__(self, config: Optional[S3Config] = None):
        """
        Initialize the S3 downloader.
        
        Args:
            config: Optional S3Config instance. If None, creates from environment.
        """
        self.config = config or S3Config()
        self._validate_config()
        self.s3_client = S3Client(self.config)
        self.file_manager = FileManager()
    
    def _validate_config(self) -> None:
        """Validate configuration and raise exception if invalid."""
        is_valid, error_msg = self.config.validate()
        if not is_valid:
            raise ValueError(error_msg)
    
    def download_md_files(self, prefix: str = '', verbose: bool = True) -> int:
        """
        Download all markdown files from S3 bucket.
        
        Args:
            prefix: Optional prefix to filter objects
            verbose: Print progress messages
        
        Returns:
            Number of files downloaded
        """
        try:
            # Ensure local directory exists
            self.file_manager.ensure_directory(self.config.local_dir)
            
            # List all .md files
            md_files = self.s3_client.list_objects(prefix=prefix, suffix='.md')
            
            if verbose:
                print(f"Found {len(md_files)} markdown files to download")
            
            # Download each file
            downloaded_count = 0
            for key in md_files:
                local_path = self.file_manager.get_local_path(
                    self.config.local_dir, key
                )
                unique_filename = self.file_manager.sanitize_filename(key)
                
                if verbose:
                    print(f"Downloading {key} -> {unique_filename}...")
                
                self.s3_client.download_file(key, local_path)
                downloaded_count += 1
            
            if verbose:
                print(f"\n✓ Successfully downloaded {downloaded_count} .md files to '{self.config.local_dir}'")
            
            return downloaded_count
        
        except Exception as e:
            if verbose:
                print(f"✗ Error: {e}")
            raise


def download_md_files() -> None:
    """
    Convenience function to download markdown files using default configuration.
    
    This function maintains backward compatibility with the original script.
    """
    try:
        downloader = S3Downloader()
        downloader.download_md_files()
    except Exception as e:
        print(f"✗ Error: {e}")


if __name__ == '__main__':
    download_md_files()
