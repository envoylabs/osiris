package store

import "os"

func Load() string {
	b, _ := os.ReadFile("frames")
	return string(b)
}
