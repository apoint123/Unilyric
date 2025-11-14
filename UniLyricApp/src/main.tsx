import { HeroUIProvider } from "@heroui/react";
import { Provider as JotaiProvider } from "jotai";
import React from "react";
import ReactDOM from "react-dom/client";
import { HashRouter } from "react-router-dom";
import App from "./App.tsx";
import "./App.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
	<React.StrictMode>
		<JotaiProvider>
			<HeroUIProvider>
				<HashRouter>
					<App />
				</HashRouter>
			</HeroUIProvider>
		</JotaiProvider>
	</React.StrictMode>,
);
