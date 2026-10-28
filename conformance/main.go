// conformance checks a running metadata provider against the book-metadata-provider contract as
// BookOrbit reads it, so an author can verify a provider without running BookOrbit.
//
//	go run ./conformance -url http://localhost:8080 -query "Dune"
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"time"
)

const (
	maxBody      = 2 << 20
	maxIconBytes = 256 << 10
)

var (
	keyPattern = regexp.MustCompile(`^[a-z][a-z0-9-]{1,31}$`)
	hexColor   = regexp.MustCompile(`^#[0-9a-fA-F]{6}$`)
	// Built-in BookOrbit providers and the pseudo-providers its clients use; no external key may be one.
	// aladin and lubimyczytac are not here: they are no longer built into BookOrbit, which keeps their ids.
	reserved = map[string]bool{
		"google": true, "goodreads": true, "amazon": true, "hardcover": true, "openlibrary": true, "itunes": true,
		"audible": true, "audnexus": true, "librofm": true, "comicvine": true, "ranobedb": true, "kobo": true,
		"googlebooks": true, "duckduckgo": true, "any": true, "all": true,
		"none": true, "auto": true, "autofill": true, "fetched": true, "file": true, "manual": true, "custom": true,
		"unknown": true, "builtin": true, "external": true,
	}
	knownFields = map[string]bool{
		"title": true, "subtitle": true, "description": true, "cover": true, "audioCover": true, "authors": true,
		"publisher": true, "isbn": true, "publishedYear": true, "language": true, "pageCount": true,
		"communityRating": true, "seriesName": true, "seriesIndex": true, "genres": true, "narrators": true,
		"duration": true, "abridged": true,
	}
)

type report struct{ failures, warnings int }

func (r *report) pass(format string, args ...any) { fmt.Printf("PASS  "+format+"\n", args...) }
func (r *report) warn(format string, args ...any) {
	r.warnings++
	fmt.Printf("WARN  "+format+"\n", args...)
}
func (r *report) fail(format string, args ...any) {
	r.failures++
	fmt.Printf("FAIL  "+format+"\n", args...)
}

func main() {
	base := flag.String("url", "http://localhost:8080", "provider base URL")
	query := flag.String("query", "Frankenstein", "a title the provider should find")
	auth := flag.String("authorization", "", "Authorization header to send, if the provider needs one")
	flag.Parse()

	r := &report{}
	client := &http.Client{Timeout: 30 * time.Second, CheckRedirect: func(req *http.Request, via []*http.Request) error {
		if len(via) >= 3 {
			return fmt.Errorf("more than 3 redirects")
		}
		return nil
	}}
	get := func(path string) (int, []byte, time.Duration, error) {
		u, err := url.JoinPath(strings.TrimRight(*base, "/")+"/", path)
		if err != nil {
			return 0, nil, 0, err
		}
		u = strings.Replace(u, "%3F", "?", 1)
		req, _ := http.NewRequest(http.MethodGet, u, nil)
		req.Header.Set("Accept", "application/json")
		if *auth != "" {
			req.Header.Set("Authorization", *auth)
		}
		start := time.Now()
		res, err := client.Do(req)
		if err != nil {
			return 0, nil, 0, err
		}
		defer res.Body.Close()
		body, err := io.ReadAll(io.LimitReader(res.Body, maxBody+1))
		return res.StatusCode, body, time.Since(start), err
	}

	info := checkInfo(r, get)
	ids := checkSearch(r, get, *query, info)
	checkLookup(r, get, info, ids)

	fmt.Printf("\n%d failure(s), %d warning(s)\n", r.failures, r.warnings)
	if r.failures > 0 {
		os.Exit(1)
	}
}

type getter func(path string) (int, []byte, time.Duration, error)

func decodeObject(r *report, name string, status int, body []byte, err error) map[string]any {
	if err != nil {
		r.fail("%s: request failed: %v", name, err)
		return nil
	}
	if len(body) > maxBody {
		r.fail("%s: body is over 2 MB; BookOrbit refuses it", name)
		return nil
	}
	if status != http.StatusOK {
		r.fail("%s: answered %d, want 200", name, status)
		return nil
	}
	var object map[string]any
	if err := json.Unmarshal(body, &object); err != nil {
		r.fail("%s: not a JSON object: %v", name, err)
		return nil
	}
	return object
}

func checkInfo(r *report, get getter) map[string]any {
	status, body, _, err := get("provider/info")
	if err == nil && status == http.StatusNotFound {
		r.warn("provider/info: missing; BookOrbit will treat the provider as search-only and name it after its address")
		return map[string]any{}
	}
	info := decodeObject(r, "provider/info", status, body, err)
	if info == nil {
		return map[string]any{}
	}
	if name, ok := info["name"].(string); !ok || strings.TrimSpace(name) == "" {
		r.fail("provider/info: name is required")
	} else {
		r.pass("provider/info: name %q", name)
	}
	if key, ok := info["key"].(string); ok {
		switch {
		case !keyPattern.MatchString(key):
			r.fail("provider/info: key %q must be 2-32 lowercase letters, digits or hyphens, starting with a letter", key)
		case reserved[strings.ToLower(key)]:
			r.fail("provider/info: key %q is reserved", key)
		default:
			r.pass("provider/info: key %q", key)
		}
	} else {
		r.warn("provider/info: no key; BookOrbit derives one from the name, and renaming the provider would change it")
	}
	if _, ok := info["version"].(string); !ok {
		r.warn("provider/info: no version; cached answers will not expire when the provider changes")
	}
	if color, present := info["color"]; present {
		if s, ok := color.(string); !ok || !hexColor.MatchString(s) {
			r.warn("provider/info: color %v is not #rrggbb; BookOrbit ignores it", color)
		} else {
			r.pass("provider/info: color %s", s)
		}
	}
	if avatar, ok := info["avatarUrl"].(string); ok && avatar != "" {
		checkIcon(r, avatar)
	}
	if fields, ok := info["supportedFields"].([]any); ok {
		for _, field := range fields {
			if name, _ := field.(string); !knownFields[name] {
				r.warn("provider/info: supportedFields has unknown field %v; it is ignored", field)
			}
		}
	}
	return info
}

// checkIcon fetches avatarUrl the way BookOrbit does: by its bytes, not its Content-Type.
func checkIcon(r *report, url string) {
	res, err := http.Get(url)
	if err != nil {
		r.warn("avatarUrl: %v; BookOrbit will show your initials instead", err)
		return
	}
	defer res.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(res.Body, maxIconBytes+1))
	switch {
	case res.StatusCode != http.StatusOK:
		r.warn("avatarUrl: answered %d; BookOrbit will show your initials instead", res.StatusCode)
	case len(body) > maxIconBytes:
		r.warn("avatarUrl: over 256 KB; BookOrbit will not keep it")
	case !rasterImage(body):
		r.warn("avatarUrl: not a PNG, WebP, JPEG, GIF or ICO image (SVG is never accepted); BookOrbit will not keep it")
	default:
		r.pass("avatarUrl: %d-byte image", len(body))
	}
}

func rasterImage(b []byte) bool {
	s := string(b)
	return strings.HasPrefix(s, "\x89PNG\r\n\x1a\n") || strings.HasPrefix(s, "\xff\xd8\xff") || strings.HasPrefix(s, "GIF87a") ||
		strings.HasPrefix(s, "GIF89a") || (len(s) >= 12 && s[:4] == "RIFF" && s[8:12] == "WEBP") || strings.HasPrefix(s, "\x00\x00\x01\x00")
}

func checkSearch(r *report, get getter, query string, info map[string]any) []string {
	mediaType := searchMediaType(info)
	status, body, took, err := get("search?query=" + url.QueryEscape(query) + "&mediaType=" + url.QueryEscape(mediaType))
	object := decodeObject(r, "search", status, body, err)
	if object == nil {
		return nil
	}
	if took > 10*time.Second {
		r.warn("search: took %s; BookOrbit gives a provider 10s by default", took.Round(time.Millisecond))
	}
	matches, ok := object["matches"].([]any)
	if !ok {
		r.fail("search: answer has no matches list")
		return nil
	}
	if len(matches) == 0 {
		r.warn("search: no matches for %q; try -query with a title the provider has", query)
	}
	if len(matches) > 50 {
		r.warn("search: %d matches; BookOrbit keeps the first 50", len(matches))
	}
	var ids []string
	for i, raw := range matches {
		record, ok := raw.(map[string]any)
		if !ok {
			r.fail("search: match %d is not an object", i)
			continue
		}
		checkRecord(r, fmt.Sprintf("search match %d", i), record)
		if id, ok := record["providerId"].(string); ok {
			ids = append(ids, id)
		}
	}
	if len(matches) > 0 {
		r.pass("search: %d match(es) in %s", len(matches), took.Round(time.Millisecond))
	}
	status, _, _, err = get("search")
	if err == nil && status != http.StatusBadRequest {
		r.warn("search without a query answered %d; the spec asks for 400", status)
	}
	return ids
}

func searchMediaType(info map[string]any) string {
	mediaTypes, ok := info["mediaTypes"].([]any)
	if !ok {
		return "book"
	}
	for _, candidate := range []string{"book", "audiobook", "comic"} {
		for _, mediaType := range mediaTypes {
			if mediaType == candidate {
				return candidate
			}
		}
	}
	return "book"
}

func checkRecord(r *report, name string, record map[string]any) {
	if id, ok := record["providerId"].(string); !ok || id == "" {
		r.fail("%s: providerId must be a non-empty string; BookOrbit drops a record without one", name)
	} else if len(id) > 512 {
		r.fail("%s: providerId is over 512 characters", name)
	}
	if title, ok := record["title"].(string); !ok || strings.TrimSpace(title) == "" {
		r.fail("%s: title is required", name)
	}
	for _, field := range []string{"authors", "narrators", "genres", "tags"} {
		if value, present := record[field]; present {
			if list, ok := value.([]any); !ok {
				r.fail("%s: %s must be a list of strings", name, field)
			} else {
				for _, entry := range list {
					if _, ok := entry.(string); !ok {
						r.fail("%s: %s must be a list of strings", name, field)
						break
					}
				}
			}
		}
	}
	for _, field := range []string{"cover", "url"} {
		if value, ok := record[field].(string); ok && !strings.HasPrefix(value, "https://") && !strings.HasPrefix(value, "http://") {
			r.fail("%s: %s must be an http(s) URL", name, field)
		}
	}
	if rating, ok := record["rating"].(float64); ok && (rating < 0 || rating > 5) {
		r.fail("%s: rating %v is outside 0-5", name, rating)
	}
	if year, present := record["publishedYear"]; present {
		if s, ok := year.(string); !ok || !regexp.MustCompile(`^\d{4}$`).MatchString(s) {
			r.warn("%s: publishedYear should be a four-digit string", name)
		}
	}
	if series, ok := record["series"].([]any); ok {
		for _, entry := range series {
			object, _ := entry.(map[string]any)
			if _, ok := object["series"].(string); !ok {
				r.fail("%s: every series entry needs a series name", name)
			}
		}
	}
}

func checkLookup(r *report, get getter, info map[string]any, ids []string) {
	if lookup, _ := info["lookup"].(bool); !lookup {
		r.warn("lookup: not declared; BookOrbit will search again on every refresh instead of looking up by id")
		return
	}
	if len(ids) > 0 {
		status, body, _, err := get("book/" + url.PathEscape(ids[0]))
		if record := decodeObject(r, "book/{id}", status, body, err); record != nil {
			checkRecord(r, "book/{id}", record)
			if record["providerId"] != ids[0] {
				r.fail("book/{id}: answered providerId %v for %q", record["providerId"], ids[0])
			} else {
				r.pass("book/{id}: found %q", ids[0])
			}
		}
	}
	status, _, _, err := get("book/" + url.PathEscape("conformance-unknown-id"))
	if err != nil {
		r.fail("book/{id}: request failed: %v", err)
	} else if status != http.StatusNotFound {
		r.fail("book/{id}: an unknown id answered %d; it must be 404 so BookOrbit knows the record is gone", status)
	} else {
		r.pass("book/{id}: unknown id answers 404")
	}
}
