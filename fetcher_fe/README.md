# PubMed Paper Fetcher

A real-time web application for searching and fetching PubMed papers with live status tracking.

## Features

- **Real-time Status Updates**: Poll every 200ms to track the fetching progress of metadata, abstract, and PDF for each paper
- **Visual Feedback**: See checkmarks for successful fetches and crosses for failures
- **Smart Retry Logic**: Automatically retry failed components up to 3 times with a retry queue
- **Modern UI**: Beautiful gradient design with smooth animations and transitions
- **Component Tracking**: Track the status of each paper component (metadata, abstract, PDF) independently

## Getting Started

### Installation

```bash
npm install
```

### Development

```bash
npm start
```

This will start the development server at `http://localhost:3000`.

### Build

```bash
npm run build
```

## How It Works

1. **Search**: Enter a PubMed query in the search bar
2. **Fetch**: The app simulates fetching metadata, abstract, and PDF for each paper
3. **Track**: Watch real-time status updates with visual indicators:
   - ⏳ Pending
   - 🔄 Fetching/Retrying
   - ✓ Success
   - ✗ Failed
4. **Retry**: Failed components are automatically added to a retry queue
5. **Complete**: All papers show their final status after maximum 3 retry attempts

## Technical Details

- **Framework**: React 18 with TypeScript
- **Build Tool**: Vite
- **Polling Interval**: 200ms
- **Max Retries**: 3 attempts per component
- **Mock Data**: Simulates realistic PubMed paper data with 30% initial failure rate

## Project Structure

```
fetcher_fe/
├── src/
│   ├── api/
│   │   └── mockApi.ts          # Mock API for simulating PubMed data
│   ├── components/
│   │   ├── PaperCard.tsx       # Individual paper display
│   │   ├── PaperList.tsx       # List of papers
│   │   ├── SearchBar.tsx       # Search input
│   │   └── StatusIndicator.tsx # Status icons
│   ├── hooks/
│   │   └── usePaperFetcher.ts  # Main fetching logic
│   ├── types.ts                # TypeScript interfaces
│   ├── App.tsx                 # Main application
│   └── main.tsx                # Entry point
└── package.json
```

## Future Enhancements

- Connect to real PubMed API
- Add filters and advanced search
- Export results to various formats
- Persistent storage of fetched papers
- Batch operations and bulk downloads
