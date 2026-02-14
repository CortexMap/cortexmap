#!/bin/bash
# Quick start script for CortexMap Fetcher Frontend

echo "🚀 Starting CortexMap Fetcher Frontend"
echo "======================================"
echo ""

# Check if node_modules exists
if [ ! -d "node_modules" ]; then
    echo "📦 Installing dependencies..."
    npm install
    echo ""
fi

# Test backend connectivity
echo "🔌 Testing backend connection..."
STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://ec2-3-88-176-142.compute-1.amazonaws.com/api/queue/status)

if [ "$STATUS" -eq 200 ]; then
    echo "✅ Backend is online"
else
    echo "⚠️  Backend returned status: $STATUS"
fi

echo ""
echo "🌐 Starting development server..."
echo "   Frontend: http://localhost:3000"
echo "   Backend:  http://ec2-3-88-176-142.compute-1.amazonaws.com"
echo ""
echo "Press Ctrl+C to stop"
echo ""

npm start
