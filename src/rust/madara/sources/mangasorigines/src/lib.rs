#![no_std]
use aidoku::{
	error::Result, prelude::*, std::current_date, std::defaults::defaults_get,
	std::net::HttpMethod, std::net::Request, std::String, std::StringRef, std::Vec, Chapter,
	DeepLink, Filter, Listing, Manga, MangaContentRating, MangaPageResult, MangaStatus,
	MangaViewer, Page,
};
use madara_template::helper::{add_user_agent_header, get_image_url};
use madara_template::template;

extern crate alloc;
use alloc::string::ToString;

const BASE_URL: &str = "https://mangas-origines.fr";
const SOURCE_PATH: &str = "oeuvre";
const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) GSA/300.0.598994205 Mobile/15E148 Safari/604";

fn get_data() -> template::MadaraSiteData {
	let data: template::MadaraSiteData = template::MadaraSiteData {
		base_url: String::from(BASE_URL),
		lang: String::from("fr"),
		source_path: String::from(SOURCE_PATH),
		date_format: String::from("dd/MM/yyyy"),
		description_selector: String::from("div.summary__content p"),
		author_selector: String::from("div.manga-authors"),
		status_filter_ongoing: String::from("En cours"),
		status_filter_completed: String::from("Terminé"),
		status_filter_cancelled: String::from("Annulé"),
		status_filter_on_hold: String::from("En pause"),
		popular: String::from("Populaire"),
		trending: String::from("Tendance"),
		alt_ajax: true,
		user_agent: Some(String::from(USER_AGENT)),
		..Default::default()
	};
	data
}

// The site's theme has been fully re-skinned away from stock Madara markup
// (custom "ori-*" classes), so the details/chapters flow is implemented
// directly here instead of going through madara_template::template, whose
// selectors no longer match. It also embeds all chapters directly on the
// details page (only CSS-hidden past the first ~50), so no ajax call is
// needed and the WAF-blocked /ajax/chapters/ endpoint is avoided entirely.

#[get_manga_list]
fn get_manga_list(filters: Vec<Filter>, page: i32) -> Result<MangaPageResult> {
	template::get_manga_list(filters, page, get_data())
}

#[get_manga_listing]
fn get_manga_listing(listing: Listing, page: i32) -> Result<MangaPageResult> {
	template::get_manga_listing(get_data(), listing, page)
}

#[get_manga_details]
fn get_manga_details(manga_id: String) -> Result<Manga> {
	let url = String::from(BASE_URL) + "/" + SOURCE_PATH + "/" + manga_id.as_str() + "/";

	let mut req = Request::new(url.as_str(), HttpMethod::Get);
	req = add_user_agent_header(req, &Some(String::from(USER_AGENT)));
	let html = req.html()?;

	let title = html.select("h1.ori-sr-title").text().read();

	let cover = get_image_url(html.select(".ori-sr-cover img").first());

	let signature = html.select(".ori-sr-signature a").array();
	let mut author = String::new();
	let mut artist = String::new();
	for (i, node) in signature.enumerate() {
		let text = node.as_node().expect("node array").text().read();
		if i == 0 {
			author = text;
		} else if i == 1 {
			artist = text;
		}
	}

	let description = html.select(".ori-sr-syn-texte").text().read();

	let mut categories: Vec<String> = Vec::new();
	for item in html.select("a.ori-sr-genre").array() {
		categories.push(item.as_node().expect("node array").text().read());
	}

	let status_text = html
		.select(".ori-sr-badge-statut")
		.text()
		.read()
		.to_lowercase();
	let status = if status_text.contains("en cours") {
		MangaStatus::Ongoing
	} else if status_text.contains("termin") {
		MangaStatus::Completed
	} else if status_text.contains("annul") {
		MangaStatus::Cancelled
	} else if status_text.contains("pause") {
		MangaStatus::Hiatus
	} else {
		MangaStatus::Unknown
	};

	// "shonen"/"seinen"/etc are demographic tags shared by both manga and
	// manhwa/manhua, so they say nothing about reading direction on their
	// own: only check them once no format tag (webtoon/manhwa/...) matched
	// any genre, and check the format tags across *all* genres first so a
	// later "Webcomic" tag isn't shadowed by an earlier "Shonen" one.
	let webtoon_tags = ["manhwa", "manhua", "webtoon", "webcomic", "vertical", "korean", "chinese"];
	let rtl_tags = ["manga", "japan"];
	let categories_lower: Vec<String> = categories.iter().map(|c| c.to_lowercase()).collect();
	let mut viewer = MangaViewer::Scroll;
	if categories_lower
		.iter()
		.any(|c| webtoon_tags.iter().any(|tag| c.contains(tag)))
	{
		viewer = MangaViewer::Scroll;
	} else if categories_lower
		.iter()
		.any(|c| rtl_tags.iter().any(|tag| c.contains(tag)))
	{
		viewer = MangaViewer::Rtl;
	}

	if let Ok(setting) = defaults_get("defaultViewer") {
		if let Ok(value) = setting.as_string() {
			viewer = match value.read().as_str() {
				"rtl" => MangaViewer::Rtl,
				"ltr" => MangaViewer::Ltr,
				"vertical" => MangaViewer::Vertical,
				"webtoon" => MangaViewer::Scroll,
				_ => viewer, // "auto" or anything else: keep the genre-based guess
			};
		}
	}

	Ok(Manga {
		id: manga_id,
		cover,
		title,
		author,
		artist,
		description,
		url,
		categories,
		status,
		nsfw: MangaContentRating::Safe,
		viewer,
	})
}

#[get_chapter_list]
fn get_chapter_list(manga_id: String) -> Result<Vec<Chapter>> {
	let url = String::from(BASE_URL) + "/" + SOURCE_PATH + "/" + manga_id.as_str() + "/";

	let mut req = Request::new(url.as_str(), HttpMethod::Get);
	req = add_user_agent_header(req, &Some(String::from(USER_AGENT)));
	let html = req.html()?;

	let date_format = "dd/MM/yy";
	let mut chapters: Vec<Chapter> = Vec::new();

	for item in html.select("div.ori-chl-row").array() {
		let row = item.as_node().expect("node array");

		let link = row.select("a.ori-chl-corps");
		let href = link.attr("href").read();
		let slash_parts: Vec<&str> = href.trim_end_matches('/').split('/').collect();
		let slug = slash_parts.last().copied().unwrap_or("").to_string();
		let id = manga_id.clone() + "/" + slug.as_str();

		let mut title = row.select(".ori-chl-nom-long").text().read();
		if title.is_empty() {
			title = row.select(".ori-chl-nom").text().read();
		}

		let chapter = row
			.attr("data-num")
			.read()
			.parse::<f32>()
			.unwrap_or(-1.0);

		let date_text = row.select(".ori-chl-date").text().read();
		let date_updated = StringRef::from(&date_text)
			.0
			.as_date(date_format, Some("fr"), None)
			.unwrap_or_else(|_| current_date());

		chapters.push(Chapter {
			id,
			title,
			volume: -1.0,
			chapter,
			date_updated,
			scanlator: String::new(),
			url: href,
			lang: String::from("fr"),
		});
	}

	Ok(chapters)
}

#[get_page_list]
fn get_page_list(_manga_id: String, chapter_id: String) -> Result<Vec<Page>> {
	template::get_page_list(chapter_id, get_data())
}

#[modify_image_request]
fn modify_image_request(request: Request) {
	template::modify_image_request(String::from("mangas-origines.fr"), request, get_data());
}

#[handle_url]
pub fn handle_url(url: String) -> Result<DeepLink> {
	template::handle_url(url, get_data())
}
