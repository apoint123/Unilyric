import {
	// ArrowDownload16Regular,
	ArrowSync16Regular,
	Home16Regular,
	Search16Regular,
	Settings16Regular,
} from "@fluentui/react-icons";
import { Input, Listbox, ListboxItem, ScrollShadow } from "@heroui/react";
import { useSetAtom } from "jotai";
import type React from "react";
import { useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { searchQueryAtom } from "../atoms";

export function Sidebar() {
	const navigate = useNavigate();
	const location = useLocation();

	const setSearchQuery = useSetAtom(searchQueryAtom);
	const [localSearch, setLocalSearch] = useState("");

	const currentKey = location.pathname.substring(1) || "home";

	const handleSearchSubmit = (e: React.FormEvent<HTMLFormElement>) => {
		e.preventDefault();
		setSearchQuery(localSearch);
		navigate("/");
	};

	const handleNavigation = (key: React.Key) => {
		const path = key === "home" ? "/" : `/${key}`;
		navigate(path);
	};

	return (
		<div className="flex flex-col h-full gap-4">
			<form onSubmit={handleSearchSubmit}>
				<Input
					aria-label="Search"
					placeholder="搜索"
					value={localSearch}
					onValueChange={setLocalSearch}
					startContent={<Search16Regular className="text-default-500" />}
				/>
			</form>

			<h1 className="text-xm font-bold px-2 mt-2">UNILYRICAPP</h1>

			<ScrollShadow hideScrollBar className="flex-1">
				<Listbox
					aria-label="Navigation"
					variant="flat"
					selectedKeys={[currentKey]}
					onAction={handleNavigation}
					itemClasses={{
						base: "gap-2",
					}}
				>
					<ListboxItem key="home" startContent={<Home16Regular />}>
						主页
					</ListboxItem>
					<ListboxItem key="converter" startContent={<ArrowSync16Regular />}>
						歌词转换
					</ListboxItem>
					{/* <ListboxItem
						key="downloads"
						startContent={<ArrowDownload16Regular />}
					>
						下载
					</ListboxItem> */}
				</Listbox>
			</ScrollShadow>

			<Listbox
				aria-label="Settings"
				variant="flat"
				selectedKeys={[currentKey]}
				onAction={handleNavigation}
				itemClasses={{
					base: "gap-2",
				}}
			>
				<ListboxItem key="settings" startContent={<Settings16Regular />}>
					设置
				</ListboxItem>
			</Listbox>
		</div>
	);
}
