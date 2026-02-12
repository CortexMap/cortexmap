"""
FetchS3 Module - A modular S3 file downloader for markdown files.

This module provides functionality to download markdown files from S3-compatible storage.
"""

from .fetchs3 import S3Downloader, download_md_files

__all__ = ['S3Downloader', 'download_md_files']
__version__ = '1.0.0'
