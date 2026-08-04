import ReactDOM from "react-dom/client";
import App from "./App";
import { loadTerminalFont } from "./terminalFont";

// Deliberately no <React.StrictMode>: StrictMode double-invokes effects in dev,
// and card effects (Task 9) spawn ptys — double-spawn is unacceptable.
const root = ReactDOM.createRoot(document.getElementById("root")!);

// Terminalfonten indlaeses FOER foerste render, ikke ved siden af den. Et kort
// kan mountes i selvsamme oejeblik appen kommer op (gendannet workspace-
// snapshot), og xterm maaler celle-bredden én gang naar kortet aabner — se
// terminalFont.ts. Rammer den maaling fallback-fonten, staar kortet med for
// mange kolonner resten af sessionen, og der er ingen billig vej tilbage.
// Prisen er en disk-laesning af tre bundlede filer, ikke et netvaerkskald, og
// den betales én gang ved opstart frem for at skulle jages pr. kort bagefter.
// loadTerminalFont() afviser aldrig: uden font renderes appen alligevel.
void loadTerminalFont().then(() => root.render(<App />));
