import { Card, CardHeader, Chip, Image, Spinner } from "@heroui/react";
import { core } from "@tauri-apps/api";
import { useAtomValue } from "jotai";
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { searchQueryAtom } from "../atoms";
import type { SearchResult } from "../types";

function getChipColorClasses(providerName: string): string {
	switch (providerName) {
		case "qq":
			return "bg-yellow-300 text-yellow-900";
		case "netease":
			return "bg-red-200 text-red-900";
		case "kugou":
			return "bg-blue-500 text-blue-1000";
		case "amll-ttml-database":
			return "bg-gray-300 text-gray-900";
		default:
			return "bg-default-200 text-default-800";
	}
}

export function HomePage() {
	const searchQuery = useAtomValue(searchQueryAtom);
	const [isLoading, setIsLoading] = useState(false);
	const [results, setResults] = useState<SearchResult[]>([]);
	const [error, setError] = useState<string | null>(null);
	const navigate = useNavigate();

	useEffect(() => {
		if (!searchQuery) {
			setResults([]);
			setError(null);
			return;
		}
		const handleSearch = async () => {
			setIsLoading(true);
			setError(null);
			try {
				const res = await core.invoke<SearchResult[]>("search_track", {
					title: searchQuery,
					artists: null,
					album: null,
				});
				setResults(res);
			} catch (err) {
				setError(err as string);
			} finally {
				setIsLoading(false);
			}
		};
		handleSearch();
	}, [searchQuery]);

	const handleCardClick = (item: SearchResult) => {
		navigate(`/song/${item.provider_name}/${item.provider_id}`, {
			state: { item },
		});
	};

	if (!searchQuery) {
		return;
	}
	if (isLoading) {
		return (
			<div className="flex h-full items-center justify-center">
				<Spinner />
			</div>
		);
	}
	if (error) {
		return (
			<Card className="bg-danger-100 border-danger-500 border">
				<div className="p-4">
					<p className="font-bold">搜索出错:</p>
					<p>{error}</p>
				</div>
			</Card>
		);
	}
	if (results.length === 0) {
		return (
			<div className="flex h-full items-center justify-center text-default-500">
				<p>未找到关于 "{searchQuery}" 的结果</p>
			</div>
		);
	}

	return (
		<div className="flex flex-col gap-2">
			<h1 className="text-2xl px-2 mb-4">
				显示“<strong className="">{searchQuery}</strong>”的搜索结果
			</h1>

			{results.map((item) => (
				<Card
					key={`${item.provider_name}-${item.provider_id}`}
					isPressable
					onPress={() => handleCardClick(item)}
				>
					<CardHeader className="flex gap-3 items-start">
						<Image
							alt="album cover"
							height={60}
							radius="sm"
							src={item.cover_url || "/vite.svg"}
							width={60}
						/>

						<div className="flex flex-col flex-1 text-left">
							<div className="flex items-center gap-2">
								<p className="text-md font-semibold truncate max-w-xs">
									{item.title}
								</p>
								<Chip
									radius="full"
									size="sm"
									classNames={{
										base: getChipColorClasses(item.provider_name),
									}}
								>
									{item.provider_name}
								</Chip>
							</div>

							<p className="text-small text-default-500">
								{item.artists.map((a) => a.name).join(", ")}
							</p>

							<p className="text-small text-default-400">
								{item.album || "N/A"}
							</p>
						</div>
					</CardHeader>
				</Card>
			))}
		</div>
	);
}
