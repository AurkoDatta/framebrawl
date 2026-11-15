import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow-condensed/600.css";
import "./style.css";

import { createRoot } from "react-dom/client";

import { startGame } from "./game";
import { Hud } from "./ui/Hud";

const mode = new URLSearchParams(location.search).has("lab") ? "lab" : "online";
const root = createRoot(document.querySelector("#hud")!);
root.render(<Hud game={null} />);
void startGame(mode).then((game) => root.render(<Hud game={game} />));
