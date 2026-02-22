#!/bin/bash

# Brain Atlas Frontend - Development Server Startup Script

echo "🧠 Brain Atlas Explorer - Starting Development Server"
echo "=================================================="
echo ""

# Check if node_modules exists
if [ ! -d "node_modules" ]; then
    echo "📦 Installing dependencies..."
    npm install
    echo ""
fi

# Check if backend is running
echo "🔍 Checking backend connection..."
if curl -s http://localhost:8080/orch/health > /dev/null 2>&1; then
    echo "✅ Backend is running on http://localhost:8080"
else
    echo "⚠️  Warning: Backend appears to be offline"
    echo "   Please ensure the orchestrator service is running on http://localhost:8080"
    echo ""
fi

echo ""
echo "🚀 Starting Vite development server..."
echo "   The application will be available at: http://localhost:3000"
echo ""
echo "=================================================="
echo ""

npm run dev
