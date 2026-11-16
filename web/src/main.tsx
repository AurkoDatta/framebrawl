import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow-condensed/600.css";
import "./style.css";

import { createRoot } from "react-dom/client";

import { startGame } from "./game";
import { Hud } from "./ui/Hud";
import type { Mode } from "./ui/store";

// A build with VITE_STATIC set has no server behind it, so everything
// runs in the tab. Any build can be asked to with ?offline.
const params = new URLSearchParams(location.search);
const serverless = import.meta.env.VITE_STATIC === "1" || params.has("offline");
const mode: Mode = params.has("replay")
  ? "replay"
  : params.has("lab")
    ? "lab"
    : serverless
      ? "offline"
      : "online";
const root = createRoot(document.querySelector("#hud")!);
root.render(<Hud game={null} />);
void startGame(mode).then((game) => root.render(<Hud game={game} />));
