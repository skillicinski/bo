package source

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"
)

func TestYouTubeURLClassification(t *testing.T) {
	match := ClassifyYouTubeURL("https://www.youtube.com/watch?v=a1mhk7mAetk")
	if match.Kind != YouTubeSupported || match.VideoID != "a1mhk7mAetk" {
		t.Fatalf("watch match = %#v", match)
	}
	match = ClassifyYouTubeURL("https://youtu.be/a1mhk7mAetk?si=x&t=30")
	if match.Kind != YouTubeSupported || match.URL != "https://youtu.be/a1mhk7mAetk?si=x&t=30" {
		t.Fatalf("short match = %#v", match)
	}
	match = ClassifyYouTubeURL("https://www.youtube.com/embed/a1mhk7mAetk")
	if match.Kind != YouTubeUnsupported || match.Reason == "" {
		t.Fatalf("embed match = %#v", match)
	}
	if ClassifyYouTubeURL("https://example.test/watch?v=a1mhk7mAetk").Kind != YouTubeNotURL {
		t.Fatal("ordinary URL classified as YouTube")
	}
}

func TestYouTubeCaptionSelection(t *testing.T) {
	track := SelectEnglishCaptionTrack([]CaptionTrack{
		{LanguageCode: "en", Kind: "asr", BaseURL: "generated"},
		{LanguageCode: "en", BaseURL: "manual"},
	})
	if track == nil || track.BaseURL != "manual" {
		t.Fatalf("selected track = %#v", track)
	}
}

func TestTranscriptParser(t *testing.T) {
	got, err := ParseTranscriptMarkdown("<timedtext><body><p><s>Hello</s><s> &amp; friends</s></p><p>Second</p></body></timedtext>")
	if err != nil || got != "Hello & friends\n\nSecond" {
		t.Fatalf("transcript = %q, err = %v", got, err)
	}
	if _, err := ParseTranscriptMarkdown("<root><title>none</title></root>"); err == nil {
		t.Fatal("empty transcript succeeded")
	}
	got, err = ParseTranscriptMarkdown(`{"events":[{"tStartMs":0,"segs":[{"utf8":"Hello"},{"utf8":" world"}]},{"tStartMs":1000,"segs":[{"utf8":"Second"}]}]}`)
	if err != nil || got != "Hello world\n\nSecond" {
		t.Fatalf("JSON3 transcript = %q, err = %v", got, err)
	}
}

func TestHTTPFetchesJSON3YouTubeTranscript(t *testing.T) {
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		header := make(http.Header)
		if request.URL.Path == "/player" {
			return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader(`{"playabilityStatus":{"status":"OK"},"videoDetails":{"title":"Video title"},"captions":{"playerCaptionsTracklistRenderer":{"captionTracks":[{"baseUrl":"https://example.test/generated","languageCode":"en","kind":"asr"},{"baseUrl":"https://example.test/captions","languageCode":"en"}]}}}`))}, nil
		}
		return &http.Response{StatusCode: http.StatusOK, Header: header, Body: io.NopCloser(strings.NewReader(`{"events":[{"segs":[{"utf8":"Hello"},{"utf8":" world"}]}]}`))}, nil
	})}
	httpSource := New(client)
	httpSource.PlayerEndpoint = "https://example.test/player"
	page, err := httpSource.Fetch(context.Background(), "https://www.youtube.com/watch?v=a1mhk7mAetk")
	if err != nil {
		t.Fatal(err)
	}
	if page.Title != "Video title" || page.Markdown != "# Video title\n\nHello world\n" || page.SourceURL != "https://www.youtube.com/watch?v=a1mhk7mAetk" {
		t.Fatalf("page = %#v", page)
	}
}
