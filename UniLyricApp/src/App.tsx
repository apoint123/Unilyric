import { Route, Routes } from "react-router-dom";
import { Sidebar } from "./components/Sidebar";
import { ConverterPage } from "./pages/Converter";
import { HomePage } from "./pages/Home";
import { SettingsPage } from "./pages/Settings";
import { SongDetailPage } from "./pages/SongDetailPage";

function App() {
	return (
		<main className="dark text-foreground bg-background flex h-screen">
			<aside className="w-60 shrink-0 bg-content1 p-4">
				<Sidebar />
			</aside>

			<main className="flex-1 overflow-y-auto bg-content2 p-9">
				<Routes>
					<Route path="/" element={<HomePage />} />
					<Route path="/converter" element={<ConverterPage />} />
					<Route path="/settings" element={<SettingsPage />} />
					<Route path="/downloads" element={<div>下载页面</div>} />
					<Route
						path="/song/:providerName/:providerId"
						element={<SongDetailPage />}
					/>
				</Routes>
			</main>
		</main>
	);
}

export default App;
