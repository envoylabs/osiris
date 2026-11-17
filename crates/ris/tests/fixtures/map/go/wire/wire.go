package wire

import "net"

func Dial() {
	net.Dial("tcp", "frames")
}
