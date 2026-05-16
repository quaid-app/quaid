import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import {
  Blocks,
  Bot,
  Database,
  FileText,
  GitBranch,
  MessageSquareText,
  Search,
  Settings2
} from "lucide-react";
import { lazy, Suspense } from "react";
import { StatusPanel } from "./components/StatusPanel";
import { ChatPage } from "./pages/ChatPage";
import { ConversationPage } from "./pages/ConversationPage";
import { FilesPage } from "./pages/FilesPage";
import { ModelPage } from "./pages/ModelPage";
import { OpsPage } from "./pages/OpsPage";

const GraphPage = lazy(() => import("./pages/GraphPage").then((module) => ({ default: module.GraphPage })));

const navItems = [
  { to: "/chat", label: "Chat", icon: Search },
  { to: "/conversation", label: "Conversations", icon: MessageSquareText },
  { to: "/models", label: "Models", icon: Bot },
  { to: "/files", label: "Files", icon: FileText },
  { to: "/graph", label: "Graph", icon: GitBranch },
  { to: "/ops", label: "Ops", icon: Settings2 }
];

export default function App() {
  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <Blocks aria-hidden="true" size={24} />
          <div>
            <strong>Quaid</strong>
            <span>Playground</span>
          </div>
        </div>
        <nav className="nav-list" aria-label="Playground sections">
          {navItems.map((item) => {
            const Icon = item.icon;
            return (
              <NavLink key={item.to} to={item.to} className={({ isActive }) => `nav-link ${isActive ? "active" : ""}`}>
                <Icon aria-hidden="true" size={18} />
                <span>{item.label}</span>
              </NavLink>
            );
          })}
        </nav>
        <StatusPanel />
      </aside>
      <main className="main-pane">
        <Routes>
          <Route path="/" element={<Navigate to="/chat" replace />} />
          <Route path="/chat" element={<ChatPage />} />
          <Route path="/conversation" element={<ConversationPage />} />
          <Route path="/models" element={<ModelPage />} />
          <Route path="/files" element={<FilesPage />} />
          <Route
            path="/graph"
            element={
              <Suspense fallback={<div className="empty-state">Loading graph...</div>}>
                <GraphPage />
              </Suspense>
            }
          />
          <Route path="/ops" element={<OpsPage />} />
        </Routes>
      </main>
      <div className="mobile-status">
        <Database aria-hidden="true" size={16} />
        <span>Local Quaid playground</span>
      </div>
    </div>
  );
}
