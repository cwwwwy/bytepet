import { render } from "preact";
import { App } from "./App";
import "../shared/theme.css";
import "./chat.css";

render(<App />, document.getElementById("app")!);
