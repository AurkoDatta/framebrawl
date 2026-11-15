import "@fontsource/barlow/400.css";
import "@fontsource/barlow/500.css";
import "@fontsource/barlow-condensed/600.css";
import "./style.css";

import { createRoot } from "react-dom/client";

import { startGame } from "./game";
import { Hud } from "./ui/Hud";

createRoot(document.querySelector("#hud")!).render(<Hud />);
void startGame();
