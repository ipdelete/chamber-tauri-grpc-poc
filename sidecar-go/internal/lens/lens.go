package lens

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	MaxHTMLBytes = 512 * 1024
	idAlphabet   = "abcdefghijklmnopqrstuvwxyz0123456789-"
)

type Manifest struct {
	Name   string `json:"name"`
	Icon   string `json:"icon"`
	View   string `json:"view"`
	Source string `json:"source"`
}

type LensSnapshot struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	Icon string `json:"icon"`
	HTML string `json:"html"`
}

func Validate(id, name, icon, html string) error {
	if len(id) == 0 || len([]byte(id)) > 64 {
		return fmt.Errorf("Lens id must use 1-64 lowercase letters, numbers, or hyphens")
	}
	if strings.HasPrefix(id, "-") || strings.HasSuffix(id, "-") {
		return fmt.Errorf("Lens id must use 1-64 lowercase letters, numbers, or hyphens")
	}
	for _, r := range id {
		if !strings.ContainsRune(idAlphabet, r) {
			return fmt.Errorf("Lens id must use 1-64 lowercase letters, numbers, or hyphens")
		}
	}

	if len(strings.TrimSpace(name)) == 0 || len([]byte(name)) > 80 {
		return fmt.Errorf("Lens name must use 1-80 characters")
	}
	if len(strings.TrimSpace(icon)) == 0 || len([]byte(icon)) > 32 {
		return fmt.Errorf("Lens icon must use 1-32 characters")
	}
	if len([]byte(html)) > MaxHTMLBytes {
		return fmt.Errorf("Lens HTML exceeds the 512 KiB limit")
	}
	lowercase := strings.ToLower(html)
	if !strings.Contains(lowercase, "<html") || !strings.Contains(lowercase, "</html>") {
		return fmt.Errorf("Lens HTML must be a complete HTML document")
	}

	return nil
}

func Write(mindRoot, id, name, icon, html string) error {
	dir := filepath.Join(mindRoot, ".github", "lens", id)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("could not create lens directory %q: %w", dir, err)
	}

	manifest := Manifest{
		Name:   name,
		Icon:   icon,
		View:   "canvas",
		Source: "index.html",
	}
	manifestBytes, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return fmt.Errorf("could not serialize lens manifest: %w", err)
	}

	if err := os.WriteFile(filepath.Join(dir, "index.html"), []byte(html), 0o644); err != nil {
		return fmt.Errorf("could not write index.html for lens %q: %w", id, err)
	}

	manifestData := append(manifestBytes, '\n')
	if err := os.WriteFile(filepath.Join(dir, "view.json"), manifestData, 0o644); err != nil {
		return fmt.Errorf("could not write view.json for lens %q: %w", id, err)
	}

	return nil
}
