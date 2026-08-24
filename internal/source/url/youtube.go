package url

import (
	"bytes"
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strings"

	xhtml "golang.org/x/net/html"

	"github.com/skillicinski/bo/internal/domain"
	internalerrors "github.com/skillicinski/bo/internal/errors"
	"github.com/skillicinski/bo/internal/source"
)

const (
	playerEndpoint        = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false"
	androidClientName     = "ANDROID"
	androidClientVersion  = "20.10.38"
	androidUserAgent      = "com.google.android.youtube/20.10.38 (Linux; U; Android 14)"
	youtubeWatchUserAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36"
)

var youtubeAPIKeyPattern = regexp.MustCompile(`(?:"|')?INNERTUBE_API_KEY(?:"|')?\s*:\s*(?:"|')([^"']+)(?:"|')`)

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

type YouTubePlugin struct {
	Client         Requester
	UserAgent      string
	PlayerEndpoint string
}

func NewYouTube(client Requester) *YouTubePlugin {
	return &YouTubePlugin{Client: client, UserAgent: androidUserAgent, PlayerEndpoint: playerEndpoint}
}

func (p *YouTubePlugin) client() Requester {
	if p == nil || p.Client == nil {
		return http.DefaultClient
	}
	return p.Client
}

func (p *YouTubePlugin) Handle(ctx context.Context, origin source.Origin) (domain.RawSnapshot, error) {
	match := ClassifyYouTubeURL(origin.Value)
	if match.Kind != YouTubeSupported {
		return domain.RawSnapshot{}, internalerrors.Source("YouTube URL: " + match.Reason)
	}
	snapshot, err := p.fetchYouTube(ctx, match)
	if err != nil {
		return domain.RawSnapshot{}, err
	}
	if origin.SourceKey != "" {
		snapshot.SourceKey = origin.SourceKey
	} else if snapshot.SourceKey == "" {
		snapshot.SourceKey = match.URL
	}
	return snapshot, nil
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

func (p *YouTubePlugin) fetchYouTube(ctx context.Context, match YouTubeURLMatch) (domain.RawSnapshot, error) {
	player, err := p.fetchYouTubePlayer(ctx, match.VideoID, "")
	if err != nil && !internalerrors.IsKind(err, internalerrors.KindSource) {
		return domain.RawSnapshot{}, err
	}
	if err == nil && SelectEnglishCaptionTrack(player.CaptionTracks()) == nil {
		err = internalerrors.Source("YouTube transcript unavailable: no English captions found")
	}
	if err != nil {
		key, keyErr := p.fetchYouTubeAPIKey(ctx, match.VideoID)
		if keyErr != nil {
			if contextErr := internalerrors.Context(keyErr); contextErr != nil {
				return domain.RawSnapshot{}, contextErr
			}
			return domain.RawSnapshot{}, err
		}
		player, err = p.fetchYouTubePlayer(ctx, match.VideoID, key)
		if err != nil {
			return domain.RawSnapshot{}, err
		}
	}

	title := match.VideoID
	if player.VideoDetails != nil && strings.TrimSpace(player.VideoDetails.Title) != "" {
		title = strings.TrimSpace(player.VideoDetails.Title)
	}
	track := SelectEnglishCaptionTrack(player.CaptionTracks())
	if track == nil {
		return domain.RawSnapshot{}, internalerrors.Source("YouTube transcript unavailable: no English captions found")
	}
	captionRequest, err := http.NewRequestWithContext(ctx, http.MethodGet, track.BaseURL, nil)
	if err != nil {
		return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindSource, "creating caption request failed", err)
	}
	captionRequest.Header.Set("User-Agent", androidUserAgent)
	captionResponse, err := p.client().Do(captionRequest)
	if err != nil {
		return domain.RawSnapshot{}, sourceFailure("YouTube network error", err)
	}
	defer captionResponse.Body.Close()
	if captionResponse.StatusCode < 200 || captionResponse.StatusCode >= 300 {
		return domain.RawSnapshot{}, internalerrors.Source(fmt.Sprintf("YouTube transcript request returned HTTP %d", captionResponse.StatusCode))
	}
	xmlBody, err := io.ReadAll(captionResponse.Body)
	if err != nil {
		return domain.RawSnapshot{}, sourceFailure("reading caption response failed", err)
	}
	body, err := ParseTranscriptMarkdown(string(xmlBody))
	if err != nil {
		return domain.RawSnapshot{}, internalerrors.Wrap(internalerrors.KindSource, "YouTube transcript parse error", err)
	}
	return domain.NewRawSnapshot(match.URL, title, []byte(fmt.Sprintf("# %s\n\n%s\n", title, body))), nil
}

func (p *YouTubePlugin) fetchYouTubePlayer(ctx context.Context, videoID, apiKey string) (*PlayerResponse, error) {
	requestBody, err := json.Marshal(map[string]any{
		"videoId": videoID,
		"context": map[string]any{"client": map[string]string{
			"clientName": androidClientName, "clientVersion": androidClientVersion, "hl": "en",
		}},
	})
	if err != nil {
		return nil, internalerrors.Wrap(internalerrors.KindSource, "encoding YouTube request failed", err)
	}
	endpoint := playerEndpoint
	if p != nil && p.PlayerEndpoint != "" {
		endpoint = p.PlayerEndpoint
	}
	if apiKey != "" {
		endpoint, err = addYouTubeAPIKey(endpoint, apiKey)
		if err != nil {
			return nil, internalerrors.Wrap(internalerrors.KindSource, "creating YouTube request failed", err)
		}
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(requestBody))
	if err != nil {
		return nil, internalerrors.Wrap(internalerrors.KindSource, "creating YouTube request failed", err)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("User-Agent", androidUserAgent)
	response, err := p.client().Do(request)
	if err != nil {
		return nil, sourceFailure("YouTube network error", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, internalerrors.Source(fmt.Sprintf("YouTube player request returned HTTP %d", response.StatusCode))
	}
	var player PlayerResponse
	if err := json.NewDecoder(response.Body).Decode(&player); err != nil {
		return nil, internalerrors.Wrap(internalerrors.KindSource, "invalid InnerTube response", err)
	}
	if err := EnsurePlayable(&player); err != nil {
		return nil, internalerrors.Wrap(internalerrors.KindSource, "YouTube player error", err)
	}
	return &player, nil
}

func (p *YouTubePlugin) fetchYouTubeAPIKey(ctx context.Context, videoID string) (string, error) {
	watchURL := "https://www.youtube.com/watch?v=" + url.QueryEscape(videoID)
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, watchURL, nil)
	if err != nil {
		return "", internalerrors.Wrap(internalerrors.KindSource, "creating YouTube watch request failed", err)
	}
	request.Header.Set("User-Agent", youtubeWatchUserAgent)
	response, err := p.client().Do(request)
	if err != nil {
		return "", sourceFailure("YouTube network error", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return "", internalerrors.Source(fmt.Sprintf("YouTube watch request returned HTTP %d", response.StatusCode))
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return "", sourceFailure("reading YouTube watch response failed", err)
	}
	key := extractYouTubeAPIKey(string(body))
	if key == "" {
		return "", internalerrors.Source("YouTube watch page has no InnerTube API key")
	}
	return key, nil
}

func addYouTubeAPIKey(endpoint, apiKey string) (string, error) {
	parsed, err := url.Parse(endpoint)
	if err != nil {
		return "", err
	}
	query := parsed.Query()
	query.Set("key", apiKey)
	parsed.RawQuery = query.Encode()
	return parsed.String(), nil
}

func extractYouTubeAPIKey(input string) string {
	match := youtubeAPIKeyPattern.FindStringSubmatch(input)
	if len(match) == 2 {
		return match[1]
	}
	return ""
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

func cleanTranscriptText(value string) string {
	return strings.Join(strings.Fields(xhtml.UnescapeString(xhtml.UnescapeString(value))), " ")
}
