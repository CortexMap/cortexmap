
from concurrent import futures
import grpc
import sys
from pathlib import Path
# Add the grpc directory to path for protobuf imports
grpc_dir = Path(__file__).parent
sys.path.insert(0, str(grpc_dir))
sys.path.append(str(Path(__file__).parent.parent))

import comm_pb2
import comm_pb2_grpc
from db.repository import search_by_query
from db.repository import get_all_responses


class BrainRegionServicer(comm_pb2_grpc.BrainRegionServiceServicer):

    def SearchBrainRegion(self, request, context):
        """Search for a brain region by query"""
        try:
            results = search_by_query(request.query)

            entries = [
                comm_pb2.BrainRegionEntry(
                    id=row['id'],
                    query=row['query'],
                    query_timestamp=int(row['query_timestamp'].timestamp() * 1000),
                    region_name=row['region_name'],
                    hemisphere=row['hemisphere'],
                    lobe=row['lobe'],
                    anatomical_region=row['anatomical_region'],
                    function_description=row['function_description'],
                    disease_description=row['disease_description'],
                    created_at=int(row['created_at'].timestamp() * 1000),
                    updated_at=int(row['updated_at'].timestamp() * 1000),
                )
                for row in results
            ]

            status = "success" if entries else "not_found"
            return comm_pb2.SearchBrainRegionResponse(
                entries=entries,
                status=status,
                error_message="" if entries else "No entries found"
            )
        except Exception as e:
            return comm_pb2.SearchBrainRegionResponse(
                entries=[],
                status="error",
                error_message=str(e)
            )

    def GetAllBrainRegions(self, request, context):
        """Retrieve all brain region entries"""
        try:
            results = get_all_responses()

            entries = [
                comm_pb2.BrainRegionEntry(
                    id=row['id'],
                    query=row['query'],
                    query_timestamp=int(row['query_timestamp'].timestamp() * 1000),
                    region_name=row['region_name'],
                    hemisphere=row['hemisphere'],
                    lobe=row['lobe'],
                    anatomical_region=row['anatomical_region'],
                    function_description=row['function_description'],
                    disease_description=row['disease_description'],
                    created_at=int(row['created_at'].timestamp() * 1000),
                    updated_at=int(row['updated_at'].timestamp() * 1000),
                )
                for row in results
            ]

            return comm_pb2.GetAllBrainRegionsResponse(
                entries=entries,
                total_count=len(entries),
                status="success",
                error_message=""
            )
        except Exception as e:
            return comm_pb2.GetAllBrainRegionsResponse(
                entries=[],
                total_count=0,
                status="error",
                error_message=str(e)
            )


def serve():
    server = grpc.server(futures.ThreadPoolExecutor(max_workers=10))
    comm_pb2_grpc.add_BrainRegionServiceServicer_to_server(
        BrainRegionServicer(), server
    )
    server.add_insecure_port('0.0.0.0:5005')
    print("gRPC server listening on port 50051...")
    server.start()
    server.wait_for_termination()


if __name__ == '__main__':
    serve()