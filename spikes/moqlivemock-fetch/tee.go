// Spike, in a scratch copy only: prints the bytes of every stream of a
// moqtransport connection as they are read and written, when MLM_TEE is set.
package tee

import (
	"context"
	"encoding/hex"
	"fmt"
	"os"
	"sync"

	"github.com/Eyevinn/moqtransport"
)

var mu sync.Mutex

func line(name, dir, kind string, id uint64, data []byte) {
	mu.Lock()
	defer mu.Unlock()
	fmt.Fprintf(os.Stderr, "TEE %s %s %s#%d %s\n", name, dir, kind, id, hex.EncodeToString(data))
}

func note(name, what, kind string, id uint64) {
	mu.Lock()
	defer mu.Unlock()
	fmt.Fprintf(os.Stderr, "TEE %s %s %s#%d\n", name, what, kind, id)
}

type conn struct {
	moqtransport.Connection
	name string
}

// Wrap returns c unchanged unless MLM_TEE is set.
func Wrap(c moqtransport.Connection, name string) moqtransport.Connection {
	if os.Getenv("MLM_TEE") == "" {
		return c
	}
	return &conn{Connection: c, name: name}
}

type recv struct {
	moqtransport.ReceiveStream
	name, kind string
}

func (r *recv) Read(p []byte) (int, error) {
	n, err := r.ReceiveStream.Read(p)
	if n > 0 {
		line(r.name, "recv", r.kind, r.StreamID(), p[:n])
	}
	if err != nil {
		note(r.name, "recv-end("+err.Error()+")", r.kind, r.StreamID())
	}
	return n, err
}

type send struct {
	moqtransport.SendStream
	name, kind string
}

func (s *send) Write(p []byte) (int, error) {
	n, err := s.SendStream.Write(p)
	if n > 0 {
		line(s.name, "send", s.kind, s.StreamID(), p[:n])
	}
	return n, err
}

func (s *send) Close() error {
	note(s.name, "send-fin", s.kind, s.StreamID())
	return s.SendStream.Close()
}

type bidi struct {
	moqtransport.Stream
	name string
}

func (b *bidi) Read(p []byte) (int, error) {
	n, err := b.Stream.Read(p)
	if n > 0 {
		line(b.name, "recv", "bidi", b.StreamID(), p[:n])
	}
	if err != nil {
		note(b.name, "recv-end("+err.Error()+")", "bidi", b.StreamID())
	}
	return n, err
}

func (b *bidi) Write(p []byte) (int, error) {
	n, err := b.Stream.Write(p)
	if n > 0 {
		line(b.name, "send", "bidi", b.StreamID(), p[:n])
	}
	return n, err
}

func (b *bidi) Close() error {
	note(b.name, "send-fin", "bidi", b.StreamID())
	return b.Stream.Close()
}

func (c *conn) AcceptStream(ctx context.Context) (moqtransport.Stream, error) {
	s, err := c.Connection.AcceptStream(ctx)
	if err != nil {
		return nil, err
	}
	return &bidi{Stream: s, name: c.name}, nil
}

func (c *conn) AcceptUniStream(ctx context.Context) (moqtransport.ReceiveStream, error) {
	s, err := c.Connection.AcceptUniStream(ctx)
	if err != nil {
		return nil, err
	}
	return &recv{ReceiveStream: s, name: c.name, kind: "uni"}, nil
}

func (c *conn) OpenStream() (moqtransport.Stream, error) {
	s, err := c.Connection.OpenStream()
	if err != nil {
		return nil, err
	}
	return &bidi{Stream: s, name: c.name}, nil
}

func (c *conn) OpenStreamSync(ctx context.Context) (moqtransport.Stream, error) {
	s, err := c.Connection.OpenStreamSync(ctx)
	if err != nil {
		return nil, err
	}
	return &bidi{Stream: s, name: c.name}, nil
}

func (c *conn) OpenUniStream() (moqtransport.SendStream, error) {
	s, err := c.Connection.OpenUniStream()
	if err != nil {
		return nil, err
	}
	return &send{SendStream: s, name: c.name, kind: "uni"}, nil
}

func (c *conn) OpenUniStreamSync(ctx context.Context) (moqtransport.SendStream, error) {
	s, err := c.Connection.OpenUniStreamSync(ctx)
	if err != nil {
		return nil, err
	}
	return &send{SendStream: s, name: c.name, kind: "uni"}, nil
}
