import ReactDOM from "react-dom/client";
import App from "./App";

// Deliberately no <React.StrictMode>: StrictMode double-invokes effects in dev,
// and card effects (Task 9) spawn ptys — double-spawn is unacceptable.
ReactDOM.createRoot(document.getElementById("root")!).render(<App />);
