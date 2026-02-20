import { BrowserRouter, Routes, Route, Navigate, Link } from 'react-router-dom';
import ConfigPage from './pages/ConfigPage';
import ChatPage from './pages/ChatPage';
import StatusPage from './pages/StatusPage';
import './App.css';

export default function App() {
  return (
    <BrowserRouter>
      <div className="app">
        <nav className="nav">
          {/* <Link to="/chat">Chat</Link> */}
          <Link to="/config">Config</Link>
          <Link to="/status">Status</Link>
        </nav>
        <Routes>
          {/* <Route path="/chat" element={<ChatPage />} /> */}
          <Route path="/config" element={<ConfigPage />} />
          <Route path="/status" element={<StatusPage />} />
          <Route path="/" element={<Navigate to="/config" replace />} />
        </Routes>
      </div>
    </BrowserRouter>
  );
}
