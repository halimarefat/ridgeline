import { createRoot } from "react-dom/client";
import "./styles.css";
import { AppProvider } from "./state";
import { Root } from "./shell";

createRoot(document.getElementById("root")!).render(
  <AppProvider>
    <Root />
  </AppProvider>,
);
