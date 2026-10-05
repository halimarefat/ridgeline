import { createRoot } from "react-dom/client";

function App() {
  return <div className="boot">Ridgeline is starting…</div>;
}

createRoot(document.getElementById("root")!).render(<App />);
