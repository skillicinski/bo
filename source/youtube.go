package source

import (
	"bytes"
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	xhtml "golang.org/x/net/html"

	"github.com/skillicinski/bo"
)

const (
	playerEndpoint       = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false"
	androidClientName    = "ANDROID"
	androidClientVersion = "20.10.38"
	androidUserAgent     = "com.google.android.youtube/20.10.38 (Linux; U; Android 14)"
)

type YouTubeURLKind uint8

const (
	YouTubeNotURL YouTubeURLKind = iota
	YouTubeSupported
	YouTubeUnsupported
)

type YouTubeURLMatch struct {
	Kind    YouTubeURLKind
	URL     string
	VideoID string
	Reason  string
}

func ClassifyYouTubeURL(input string) YouTubeURLMatch {
	parsed, err := url.Parse(input)
	if err != nil {
		return YouTubeURLMatch{Kind: YouTubeNotURL}
	}
	host := strings.ToLower(parsed.Hostname())
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		if youtubeLikeHost(host) {
			return unsupportedYouTube(parsed.String(), fmt.Sprintf("scheme '%s' is not supported", parsed.Scheme))
		}
		return YouTubeURLMatch{Kind: YouTubeNotURL}
	}
	switch host {
	case "youtu.be":
		return classifyShortURL(parsed)
	case "youtube.com", "www.youtube.com":
		return classifyYouTubeHost(parsed)
	default:
		if youtubeLikeHost(host) {
			return unsupportedYouTube(parsed.String(), fmt.Sprintf("host '%s' is not supported", host))
		}
		return YouTubeURLMatch{Kind: YouTubeNotURL}
	}
}

func classifyShortURL(parsed *url.URL) YouTubeURLMatch {
	value := strings.Trim(parsed.Path, "/")
	if err := validVideoID(value); err != nil {
		return unsupportedYouTube(parsed.String(), err.Error())
	}
	return YouTubeURLMatch{Kind: YouTubeSupported, URL: parsed.String(), VideoID: value}
}

func classifyYouTubeHost(parsed *url.URL) YouTubeURLMatch {
	path := parsed.Path
	switch {
	case path == "/watch":
		videoID := parsed.Query().Get("v")
		if videoID == "" {
			return unsupportedYouTube(parsed.String(), "watch URL is missing v parameter")
		}
		if err := validVideoID(videoID); err != nil {
			return unsupportedYouTube(parsed.String(), err.Error())
		}
		return YouTubeURLMatch{Kind: YouTubeSupported, URL: parsed.String(), VideoID: videoID}
	case strings.HasPrefix(path, "/shorts/"):
		videoID := strings.Trim(strings.TrimPrefix(path, "/shorts/"), "/")
		if err := validVideoID(videoID); err != nil {
			return unsupportedYouTube(parsed.String(), err.Error())
		}
		return YouTubeURLMatch{Kind: YouTubeSupported, URL: parsed.String(), VideoID: videoID}
	case strings.HasPrefix(path, "/embed/"):
		return unsupportedYouTube(parsed.String(), "embed URLs are not collected; collect the containing page or original video URL")
	case strings.HasPrefix(path, "/playlist"):
		return unsupportedYouTube(parsed.String(), "playlist collection is out of scope")
	case strings.HasPrefix(path, "/channel/"), strings.HasPrefix(path, "/@"):
		return unsupportedYouTube(parsed.String(), "channel collection is out of scope")
	case strings.HasPrefix(path, "/results"):
		return unsupportedYouTube(parsed.String(), "search result collection is out of scope")
	default:
		return unsupportedYouTube(parsed.String(), "not a supported YouTube video URL")
	}
}

func unsupportedYouTube(input, reason string) YouTubeURLMatch {
	return YouTubeURLMatch{Kind: YouTubeUnsupported, URL: input, Reason: reason}
}

func validVideoID(value string) error {
	if value == "" {
		return fmt.Errorf("video ID is missing")
	}
	if len(value) != 11 {
		return fmt.Errorf("video ID must be 11 URL-safe characters")
	}
	for _, r := range value {
		if !(r >= 'a' && r <= 'z') && !(r >= 'A' && r <= 'Z') && !(r >= '0' && r <= '9') && r != '_' && r != '-' {
			return fmt.Errorf("video ID contains unsupported characters")
		}
	}
	return nil
}

func youtubeLikeHost(host string) bool {
	switch host {
	case "youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "www.youtu.be", "youtube-nocookie.com", "www.youtube-nocookie.com":
		return true
	default:
		return false
	}
}

type PlayerResponse struct {
	PlayabilityStatus *PlayabilityStatus `json:"playabilityStatus"`
	VideoDetails      *VideoDetails      `json:"videoDetails"`
	Captions          *Captions          `json:"captions"`
}

type PlayabilityStatus struct {
	Status string `json:"status"`
	Reason string `json:"reason"`
}

type VideoDetails struct {
	Title string `json:"title"`
}

type Captions struct {
	PlayerCaptionsTracklistRenderer *PlayerCaptionsTracklistRenderer `json:"playerCaptionsTracklistRenderer"`
}

type PlayerCaptionsTracklistRenderer struct {
	CaptionTracks []CaptionTrack `json:"captionTracks"`
}

type CaptionTrack struct {
	BaseURL      string `json:"baseUrl"`
	LanguageCode string `json:"languageCode"`
	Kind         string `json:"kind"`
}

func (p *PlayerResponse) CaptionTracks() []CaptionTrack {
	if p == nil || p.Captions == nil || p.Captions.PlayerCaptionsTracklistRenderer == nil {
		return nil
	}
	return p.Captions.PlayerCaptionsTracklistRenderer.CaptionTracks
}

func EnsurePlayable(player *PlayerResponse) error {
	if player == nil || player.PlayabilityStatus == nil || player.PlayabilityStatus.Status == "" {
		return fmt.Errorf("missing playability status")
	}
	if player.PlayabilityStatus.Status != "OK" {
		if player.PlayabilityStatus.Reason != "" {
			return fmt.Errorf("%s", player.PlayabilityStatus.Reason)
		}
		return fmt.Errorf("playability status is %s", player.PlayabilityStatus.Status)
	}
	return nil
}

func SelectEnglishCaptionTrack(tracks []CaptionTrack) *CaptionTrack {
	for _, generated := range []bool{false, true} {
		for index := range tracks {
			track := &tracks[index]
			if !isEnglish(track.LanguageCode) || (track.Kind == "asr") != generated || track.BaseURL == "" {
				continue
			}
			copy := *track
			return &copy
		}
	}
	return nil
}

func isEnglish(language string) bool { return language == "en" || strings.HasPrefix(language, "en-") }

func (h *HTTP) fetchYouTube(ctx context.Context, match YouTubeURLMatch) (bo.Page, error) {
	requestBody, err := json.Marshal(map[string]any{
		"videoId": match.VideoID,
		"context": map[string]any{"client": map[string]string{
			"clientName": androidClientName, "clientVersion": androidClientVersion, "hl": "en",
		}},
	})
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("encoding YouTube request failed: %v", err))
	}
	endpoint := playerEndpoint
	if h != nil && h.PlayerEndpoint != "" {
		endpoint = h.PlayerEndpoint
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(requestBody))
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("creating YouTube request failed: %v", err))
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("User-Agent", androidUserAgent)
	response, err := h.client().Do(request)
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("YouTube network error: %v", err))
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return bo.Page{}, bo.HTTPError(response.StatusCode, response.Header.Get("X-Request-Id"))
	}
	var player PlayerResponse
	if err := json.NewDecoder(response.Body).Decode(&player); err != nil {
		return bo.Page{}, bo.ContentError(fmt.Sprintf("invalid InnerTube response: %v", err))
	}
	if err := EnsurePlayable(&player); err != nil {
		return bo.Page{}, bo.ContentError("YouTube player error: " + err.Error())
	}
	title := match.VideoID
	if player.VideoDetails != nil && strings.TrimSpace(player.VideoDetails.Title) != "" {
		title = strings.TrimSpace(player.VideoDetails.Title)
	}
	track := SelectEnglishCaptionTrack(player.CaptionTracks())
	if track == nil {
		return bo.Page{}, bo.ContentError("YouTube transcript unavailable: no English captions found")
	}
	captionRequest, err := http.NewRequestWithContext(ctx, http.MethodGet, track.BaseURL, nil)
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("creating caption request failed: %v", err))
	}
	captionRequest.Header.Set("User-Agent", androidUserAgent)
	captionResponse, err := h.client().Do(captionRequest)
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("YouTube network error: %v", err))
	}
	defer captionResponse.Body.Close()
	if captionResponse.StatusCode < 200 || captionResponse.StatusCode >= 300 {
		return bo.Page{}, bo.HTTPError(captionResponse.StatusCode, captionResponse.Header.Get("X-Request-Id"))
	}
	xmlBody, err := io.ReadAll(captionResponse.Body)
	if err != nil {
		return bo.Page{}, bo.RequestError(fmt.Sprintf("reading caption response failed: %v", err))
	}
	body, err := ParseTranscriptMarkdown(string(xmlBody))
	if err != nil {
		return bo.Page{}, bo.ContentError("YouTube transcript parse error: " + err.Error())
	}
	return bo.Page{Title: title, Markdown: fmt.Sprintf("# %s\n\n%s\n", title, body), SourceURL: match.URL}, nil
}

func ParseTranscriptMarkdown(input string) (string, error) {
	if strings.HasPrefix(strings.TrimSpace(input), "{") {
		return parseJSON3Transcript(input)
	}
	decoder := xml.NewDecoder(strings.NewReader(input))
	segments := []string{}
	current := ""
	active := ""
	depth := 0
	sawTranscriptTag := false
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", err
		}
		switch value := token.(type) {
		case xml.StartElement:
			if active == "" && (value.Name.Local == "p" || value.Name.Local == "text") {
				active = value.Name.Local
				current = ""
				sawTranscriptTag = true
			} else if active != "" {
				depth++
			}
		case xml.CharData:
			if active != "" {
				current += string(value)
			}
		case xml.EndElement:
			if active == value.Name.Local && depth == 0 {
				if cleaned := cleanTranscriptText(current); cleaned != "" {
					segments = append(segments, cleaned)
				}
				active = ""
				current = ""
			} else if active != "" && depth > 0 {
				depth--
			}
		}
	}
	if !sawTranscriptTag || len(segments) == 0 {
		return "", fmt.Errorf("transcript is empty")
	}
	return strings.Join(segments, "\n\n"), nil
}

func parseJSON3Transcript(input string) (string, error) {
	var transcript struct {
		Events []struct {
			Segments []struct {
				Text string `json:"utf8"`
			} `json:"segs"`
		} `json:"events"`
	}
	if err := json.Unmarshal([]byte(input), &transcript); err != nil {
		return "", err
	}
	segments := make([]string, 0, len(transcript.Events))
	for _, event := range transcript.Events {
		var builder strings.Builder
		for _, segment := range event.Segments {
			builder.WriteString(segment.Text)
		}
		if cleaned := cleanTranscriptText(builder.String()); cleaned != "" {
			segments = append(segments, cleaned)
		}
	}
	if len(segments) == 0 {
		return "", fmt.Errorf("transcript is empty")
	}
	return strings.Join(segments, "\n\n"), nil
}

func ParseJSON3Transcript(input string) (string, error) { return parseJSON3Transcript(input) }

func cleanTranscriptText(value string) string {
	return strings.Join(strings.Fields(xhtml.UnescapeString(xhtml.UnescapeString(value))), " ")
}
