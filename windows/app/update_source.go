package main

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"

	"github.com/Masterminds/semver/v3"
	selfupdate "github.com/creativeprojects/go-selfupdate"
	"github.com/google/go-github/v93/github"
)

// The release contains only the installer. Adapt GitHub's authenticated
// asset digest to the library's SHA validator without a second public asset.
type githubUpdates struct {
	api     *github.Client
	http    *http.Client
	mu      sync.Mutex
	digests map[int64]string
}

type releaseWithDigest struct{ selfupdate.SourceRelease }
type assetDigest struct{ selfupdate.SourceAsset }

func (asset assetDigest) GetID() int64    { return -asset.SourceAsset.GetID() }
func (asset assetDigest) GetName() string { return asset.SourceAsset.GetName() + ".sha256" }
func (asset assetDigest) GetSize() int    { return 64 }

func (release releaseWithDigest) GetAssets() []selfupdate.SourceAsset {
	assets := release.SourceRelease.GetAssets()
	out := make([]selfupdate.SourceAsset, 0, len(assets)*2)
	for _, asset := range assets {
		out = append(out, asset, assetDigest{asset})
	}
	return out
}

func newUpdateSource(client *http.Client, base string) (*githubUpdates, error) {
	options := []github.ClientOptionsFunc{github.WithHTTPClient(client)}
	if base != "" { // Local HTTP fixture; production keeps GitHub's default URL.
		options = append(options, github.WithURLs(&base, &base))
	}
	api, err := github.NewClient(options...)
	if err != nil {
		return nil, err
	}
	return &githubUpdates{api: api, http: client, digests: make(map[int64]string)}, nil
}

func (source *githubUpdates) ListReleases(ctx context.Context, _ selfupdate.Repository) ([]selfupdate.SourceRelease, error) {
	var releases []selfupdate.SourceRelease
	for release, err := range source.api.Repositories.ListReleasesIter(ctx, "OUBIGFA", "FreeType", nil) {
		if err != nil {
			return nil, err
		}
		version, parseErr := semver.StrictNewVersion(strings.TrimPrefix(release.GetTagName(), "freetype-"))
		if parseErr != nil || version.Prerelease() != "" || version.Metadata() != "" {
			continue
		}
		// A tag and its installer version must agree; unrelated assets stay out.
		name := "FreeType-" + version.String() + "-Setup.exe"
		var assets []*github.ReleaseAsset
		for _, asset := range release.Assets {
			if asset.GetName() == name && asset.GetID() > 0 && asset.GetSize() > 0 {
				assets = append(assets, asset)
			}
		}
		release.Assets = assets
		releases = append(releases, releaseWithDigest{selfupdate.NewGitHubRelease(release)})
	}
	return releases, nil
}

func (source *githubUpdates) DownloadReleaseAsset(ctx context.Context, _ *selfupdate.Release, id int64) (io.ReadCloser, error) {
	if id < 0 {
		asset, _, err := source.api.Repositories.GetReleaseAsset(ctx, "OUBIGFA", "FreeType", -id)
		if err != nil {
			return nil, err
		}
		digest := asset.GetDigest()
		if !strings.HasPrefix(digest, "sha256:") {
			return nil, fmt.Errorf("GitHub 未提供安装器的 SHA-256 校验值")
		}
		digest = strings.TrimPrefix(digest, "sha256:")
		source.mu.Lock()
		source.digests[-id] = digest
		source.mu.Unlock()
		return io.NopCloser(strings.NewReader(digest)), nil
	}
	reader, _, err := source.api.Repositories.DownloadReleaseAsset(ctx, "OUBIGFA", "FreeType", id, source.http)
	return reader, err
}
