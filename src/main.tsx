import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// No <StrictMode>: its dev-only double mount would spawn the shell twice.
ReactDOM.createRoot(document.getElementById("root")!).render(<App />);
