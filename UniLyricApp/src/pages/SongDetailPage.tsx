import {
	ArrowLeft16Regular,
	ChevronRight16Regular,
} from "@fluentui/react-icons";
import {
	Avatar,
	Button,
	Divider,
	Image,
	Modal,
	ModalBody,
	ModalContent,
	Spinner,
	useDisclosure,
} from "@heroui/react";
import { core } from "@tauri-apps/api";
import { useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { SearchResult, ViewableLyrics } from "../types";

export function SongDetailPage() {
	const location = useLocation();
	const navigate = useNavigate();

	const songData = location.state?.item as SearchResult | undefined;

	const [lyrics, setLyrics] = useState<ViewableLyrics | null>(null);
	const [isLoading, setIsLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);

	const { isOpen, onOpen, onOpenChange } = useDisclosure();

	useEffect(() => {
		if (!songData) {
			navigate("/");
			return;
		}

		const fetchLyrics = async () => {
			setIsLoading(true);
			setError(null);
			try {
				const result = await core.invoke<ViewableLyrics>("get_full_lyrics", {
					providerName: songData.provider_name,
					songId: songData.provider_id,
				});
				setLyrics(result);
			} catch (err) {
				setError(err as string);
			} finally {
				setIsLoading(false);
			}
		};

		fetchLyrics();
	}, [songData, navigate]);

	if (!songData) {
		return null;
	}

	const lyricPreviewLine1 =
		lyrics?.lines[0]?.mainText || (isLoading ? "..." : "");
	const lyricPreviewLine2 =
		lyrics?.lines[1]?.mainText || (isLoading ? "..." : "");

	return (
		<>
			<Button
				isIconOnly
				variant="flat"
				onPress={() => navigate(-1)}
				className="self-start mb-5"
			>
				<ArrowLeft16Regular />
			</Button>

			<div className="grid grid-cols-[auto_1fr] gap-x-7">
				<Image
					width={300}
					height={300}
					radius="sm"
					src={songData.cover_url || "/vite.svg"}
					alt="Album Cover"
				/>
				<div className="flex flex-col text-left justify-center">
					<h1 className="text-3xl font-bold">{songData.title}</h1>
					<p className="text-xl text-default-500">
						{songData.artists.map((a) => a.name).join(", ")}
					</p>
					<p className="text-lg text-default-400">{songData.album || "N/A"}</p>
				</div>

				<Divider className="col-span-2 my-6" />

				<h2 className="text-xl font-bold col-span-2">歌词</h2>

				<div></div>
				<div className="flex flex-col items-start gap-2 mt-2">
					<div className="text-left text-lg">
						{isLoading && <Spinner size="sm" />}
						{error && <p className="text-danger-500">歌词加载失败</p>}
						{!isLoading && !error && (
							<>
								<p>{lyricPreviewLine1}</p>
								<p>{lyricPreviewLine2}</p>
							</>
						)}
					</div>
					<Button
						variant="light"
						color="primary"
						onPress={onOpen}
						endContent={<ChevronRight16Regular />}
						className="p-0"
						isDisabled={isLoading || !!error}
					>
						查看完整歌词
					</Button>
				</div>

				<Divider className="col-span-2 my-6" />

				<h2 className="text-xl font-bold col-span-2">出演艺人</h2>

				<div></div>
				<div className="flex flex-col gap-2 w-full mt-2">
					{songData.artists.map((artist) => (
						<div
							key={artist.id}
							className="flex items-center gap-3 p-2 rounded-lg"
						>
							<Avatar name={artist.name} />
							<p className="font-semibold">{artist.name}</p>
						</div>
					))}
				</div>
			</div>

			<Modal
				isOpen={isOpen}
				onOpenChange={onOpenChange}
				size="2xl"
				scrollBehavior="inside"
			>
				<ModalContent>
					<ModalBody>
						<pre className="text-left">{lyrics?.rawText}</pre>
					</ModalBody>
				</ModalContent>
			</Modal>
		</>
	);
}
