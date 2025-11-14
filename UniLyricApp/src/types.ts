export interface Artist {
	id: string;
	name: string;
}

export type MatchType =
	| "None"
	| "VeryLow"
	| "Low"
	| "Medium"
	| "PrettyHigh"
	| "High"
	| "VeryHigh"
	| "Perfect";

export type Language =
	| "Instrumental"
	| "Chinese"
	| "English"
	| "Japanese"
	| "Korean"
	| "Other";

export interface SearchResult {
	title: string;
	artists: Artist[];
	album: string | null;
	album_id: string | null;
	duration: number | null;
	provider_id: string;
	provider_name: string;
	provider_id_num: number | null;
	match_type: MatchType;
	cover_url: string | null;
	language: Language | null;
}

export interface SimpleLyricLine {
	startMs: number;
	mainText: string | null;
	translationText: string | null;
	romanizationText: string | null;
	agent: string | null;
}

export interface ViewableLyrics {
	lines: SimpleLyricLine[];
	rawText: string;
	availableTranslations: string[];
	availableRomanizations: string[];
}
