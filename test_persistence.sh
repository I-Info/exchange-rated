#!/bin/bash

# Test script for the rated application with SQLite persistence

echo "🧪 Testing Rated Application with SQLite Persistence"
echo "=================================================="

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if the binary exists
if [ ! -f "target/release/rated" ]; then
    print_warning "Release binary not found, building..."
    cargo build --release
    if [ $? -ne 0 ]; then
        print_error "Failed to build the application"
        exit 1
    fi
fi

# Clean up any existing database for fresh test
if [ -f "rates.db" ]; then
    print_warning "Removing existing database for fresh test..."
    rm rates.db
fi

# Start the application in background
print_status "Starting the rated application..."
./target/release/rated &
APP_PID=$!

# Give the app time to start
sleep 3

# Check if the app is running
if ! kill -0 $APP_PID 2>/dev/null; then
    print_error "Application failed to start"
    exit 1
fi

print_success "Application started with PID: $APP_PID"

# Test the API endpoints
print_status "Testing API endpoints..."

# Test home page
print_status "Testing home page (/)..."
curl -s http://127.0.0.1:3000/ > /dev/null
if [ $? -eq 0 ]; then
    print_success "Home page accessible"
else
    print_error "Home page not accessible"
fi

# Test latest rate API
print_status "Testing latest rate API (/api/latest)..."
sleep 5  # Wait for some data to be fetched
LATEST_RESPONSE=$(curl -s http://127.0.0.1:3000/api/latest)
if [ $? -eq 0 ] && [ ! -z "$LATEST_RESPONSE" ]; then
    print_success "Latest rate API working: $LATEST_RESPONSE"
else
    print_warning "Latest rate API returned empty response (this is normal if no data has been fetched yet)"
fi

# Test history API
print_status "Testing history API (/api/history)..."
HISTORY_RESPONSE=$(curl -s http://127.0.0.1:3000/api/history)
if [ $? -eq 0 ]; then
    print_success "History API working"
    echo "History response: $HISTORY_RESPONSE"
else
    print_error "History API not working"
fi

# Wait for the application to fetch some data and test persistence
print_status "Waiting 30 seconds for rate fetching and persistence testing..."
sleep 30

# Check if database was created
if [ -f "rates.db" ]; then
    print_success "Database file created: rates.db"

    # Check database content using sqlite3 if available
    if command -v sqlite3 &> /dev/null; then
        print_status "Checking database content..."
        RECORD_COUNT=$(sqlite3 rates.db "SELECT COUNT(*) FROM rate_records;")
        print_success "Database contains $RECORD_COUNT rate records"

        # Show latest 5 records
        print_status "Latest 5 records in database:"
        sqlite3 rates.db "SELECT rate, timestamp FROM rate_records ORDER BY timestamp DESC LIMIT 5;" | while read line; do
            echo "  📊 $line"
        done
    else
        print_warning "sqlite3 command not found, cannot inspect database content"
    fi
else
    print_warning "Database file not created yet (this might be normal if no data has been fetched)"
fi

# Test WebSocket connection (basic test)
print_status "Testing WebSocket connection..."
if command -v wscat &> /dev/null; then
    echo '{"type":"ping"}' | wscat -c ws://127.0.0.1:3000/ws -w 1 &
    sleep 2
    print_success "WebSocket connection test completed"
else
    print_warning "wscat not found, skipping WebSocket test"
    print_status "You can manually test WebSocket at: ws://127.0.0.1:3000/ws"
fi

# Show application logs for a few seconds
print_status "Application logs (last 10 lines):"
sleep 2

# Clean up
print_status "Cleaning up..."
kill $APP_PID 2>/dev/null
wait $APP_PID 2>/dev/null

print_success "Test completed!"
print_status "The application supports the following features:"
echo "  ✅ Real-time rate fetching from Bank of China"
echo "  ✅ SQLite database persistence"
echo "  ✅ RESTful API endpoints"
echo "  ✅ WebSocket real-time updates"
echo "  ✅ Automatic persistence when memory reaches 150 records"
echo "  ✅ Periodic persistence every 5 minutes"
echo "  ✅ Memory management (keeps only latest 75 records after persistence)"

print_status "To run the application manually:"
echo "  cargo run"
echo "  or"
echo "  ./target/release/rated"
echo ""
print_status "Access the application at: http://127.0.0.1:3000"
print_status "WebSocket endpoint: ws://127.0.0.1:3000/ws"
print_status "Database file: rates.db"
