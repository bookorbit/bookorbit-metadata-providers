package main

import "testing"

func TestSearchMediaType(t *testing.T) {
	testCases := []struct {
		name string
		info map[string]any
		want string
	}{
		{name: "book first", info: map[string]any{"mediaTypes": []any{"audiobook", "book"}}, want: "book"},
		{name: "audio only", info: map[string]any{"mediaTypes": []any{"audiobook"}}, want: "audiobook"},
		{name: "comic only", info: map[string]any{"mediaTypes": []any{"comic"}}, want: "comic"},
		{name: "default", info: map[string]any{}, want: "book"},
	}
	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			if got := searchMediaType(testCase.info); got != testCase.want {
				t.Fatalf("searchMediaType() = %q, want %q", got, testCase.want)
			}
		})
	}
}
