"""
Test client for Brain Atlas HTTP API with binary protobuf.

This script demonstrates how to interact with the HTTP API using
binary protobuf serialization/deserialization.
"""

import sys
from pathlib import Path

# Add parent directories to path
sys.path.append(str(Path(__file__).parent.parent))
sys.path.append(str(Path(__file__).parent.parent / "grpc"))

import requests
import comm_pb2


def test_search_brain_region(query: str, host: str = "localhost", port: int = 5006):
    """
    Test the search-brain-region endpoint.
    
    Args:
        query: Search query string
        host: Server host
        port: Server port
    """
    print(f"\n{'='*60}")
    print(f"Testing SearchBrainRegion with query: '{query}'")
    print(f"{'='*60}")
    
    # Create and serialize request
    request = comm_pb2.SearchBrainRegionRequest()
    request.query = query
    request_data = request.SerializeToString()
    
    print(f"Request size: {len(request_data)} bytes")
    
    # Send HTTP POST request
    url = f"http://{host}:{port}/search-brain-region"
    headers = {"Content-Type": "application/x-protobuf"}
    
    try:
        response = requests.post(url, data=request_data, headers=headers)
        print(f"HTTP Status: {response.status_code}")
        print(f"Response size: {len(response.content)} bytes")
        
        # Deserialize response
        pb_response = comm_pb2.SearchBrainRegionResponse()
        pb_response.ParseFromString(response.content)
        
        print(f"\nStatus: {pb_response.status}")
        print(f"Found {len(pb_response.entries)} entries")
        
        if pb_response.error_message:
            print(f"Error message: {pb_response.error_message}")
        
        # Display entries
        for i, entry in enumerate(pb_response.entries, 1):
            print(f"\n--- Entry {i} ---")
            print(f"ID: {entry.id}")
            print(f"Region Name: {entry.region_name}")
            print(f"Hemisphere: {entry.hemisphere}")
            print(f"Lobe: {entry.lobe}")
            print(f"Anatomical Region: {entry.anatomical_region}")
            print(f"Query: {entry.query}")
            print(f"Function: {entry.function_description[:100]}..." if len(entry.function_description) > 100 else f"Function: {entry.function_description}")
            print(f"Disease: {entry.disease_description[:100]}..." if len(entry.disease_description) > 100 else f"Disease: {entry.disease_description}")
        
        return pb_response
        
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None
    except Exception as e:
        print(f"Error: {e}")
        return None


def test_get_all_brain_regions(host: str = "localhost", port: int = 5006):
    """
    Test the get-all-brain-regions endpoint.
    
    Args:
        host: Server host
        port: Server port
    """
    print(f"\n{'='*60}")
    print("Testing GetAllBrainRegions")
    print(f"{'='*60}")
    
    # Create and serialize request (empty message)
    request = comm_pb2.GetAllBrainRegionsRequest()
    request_data = request.SerializeToString()
    
    print(f"Request size: {len(request_data)} bytes")
    
    # Send HTTP POST request
    url = f"http://{host}:{port}/get-all-brain-regions"
    headers = {"Content-Type": "application/x-protobuf"}
    
    try:
        response = requests.post(url, data=request_data, headers=headers)
        print(f"HTTP Status: {response.status_code}")
        print(f"Response size: {len(response.content)} bytes")
        
        # Deserialize response
        pb_response = comm_pb2.GetAllBrainRegionsResponse()
        pb_response.ParseFromString(response.content)
        
        print(f"\nStatus: {pb_response.status}")
        print(f"Total count: {pb_response.total_count}")
        print(f"Entries returned: {len(pb_response.entries)}")
        
        if pb_response.error_message:
            print(f"Error message: {pb_response.error_message}")
        
        # Display first few entries
        display_count = min(3, len(pb_response.entries))
        if display_count > 0:
            print(f"\nShowing first {display_count} entries:")
            for i, entry in enumerate(pb_response.entries[:display_count], 1):
                print(f"\n--- Entry {i} ---")
                print(f"ID: {entry.id}")
                print(f"Region Name: {entry.region_name}")
                print(f"Hemisphere: {entry.hemisphere}")
                print(f"Lobe: {entry.lobe}")
        
        return pb_response
        
    except requests.exceptions.RequestException as e:
        print(f"Request failed: {e}")
        return None
    except Exception as e:
        print(f"Error: {e}")
        return None


def test_health_check(host: str = "localhost", port: int = 5006):
    """
    Test the health check endpoint.
    
    Args:
        host: Server host
        port: Server port
    """
    print(f"\n{'='*60}")
    print("Testing Health Check")
    print(f"{'='*60}")
    
    url = f"http://{host}:{port}/health"
    
    try:
        response = requests.get(url)
        print(f"HTTP Status: {response.status_code}")
        print(f"Response: {response.json()}")
        return response.json()
    except Exception as e:
        print(f"Error: {e}")
        return None


def test_invalid_content_type(host: str = "localhost", port: int = 5006):
    """
    Test Content-Type validation.
    
    Args:
        host: Server host
        port: Server port
    """
    print(f"\n{'='*60}")
    print("Testing Invalid Content-Type (should fail with 415)")
    print(f"{'='*60}")
    
    request = comm_pb2.SearchBrainRegionRequest()
    request.query = "test"
    request_data = request.SerializeToString()
    
    url = f"http://{host}:{port}/search-brain-region"
    headers = {"Content-Type": "application/json"}  # Wrong content type
    
    try:
        response = requests.post(url, data=request_data, headers=headers)
        print(f"HTTP Status: {response.status_code}")
        print(f"Response: {response.json()}")
        return response
    except Exception as e:
        print(f"Error: {e}")
        return None


def main():
    """Run all tests."""
    import argparse
    
    parser = argparse.ArgumentParser(description="Test Brain Atlas HTTP API")
    parser.add_argument("--host", default="localhost", help="Server host")
    parser.add_argument("--port", type=int, default=5006, help="Server port")
    parser.add_argument("--query", default="hippocampus", help="Search query")
    
    args = parser.parse_args()
    
    print("\n" + "="*60)
    print("Brain Atlas HTTP API Test Client")
    print("="*60)
    print(f"Server: http://{args.host}:{args.port}")
    print("="*60)
    
    # Run tests
    test_health_check(args.host, args.port)
    test_search_brain_region(args.query, args.host, args.port)
    test_get_all_brain_regions(args.host, args.port)
    test_invalid_content_type(args.host, args.port)
    
    print("\n" + "="*60)
    print("Tests completed!")
    print("="*60 + "\n")


if __name__ == '__main__':
    main()
