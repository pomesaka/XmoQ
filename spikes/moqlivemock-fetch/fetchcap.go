// Spike, in a scratch copy only: a publisher that answers FETCH with scripted
// records through Eyevinn/moqtransport, and a client that sends a Standalone
// FETCH. With MLM_TEE=1 the bytes of every stream are printed (internal/tee).
// The encoding of every byte is the library's; only which records are written
// is decided here.
package main

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"os"
	"strconv"
	"time"

	"github.com/Eyevinn/moqlivemock/internal"
	"github.com/Eyevinn/moqlivemock/internal/tee"
	"github.com/Eyevinn/moqtransport"
	"github.com/Eyevinn/moqtransport/quicmoq"
	"github.com/quic-go/quic-go"
)

type record struct {
	end      moqtransport.EndOfRange
	group    uint64
	object   uint64
	subgroup uint64
	datagram bool
	priority uint8
	payload  []byte
}

func obj(g, o, sub uint64, prio uint8, payload ...byte) record {
	return record{group: g, object: o, subgroup: sub, priority: prio, payload: payload}
}

// The same values as the files of fixtures/moqt-18-by-hand/ of XmoQ.
var scripts = map[string][]record{
	"descending": {obj(9, 0, 0, 7, 0xaa), obj(7, 1, 0, 7, 0xbb), obj(6, 5, 0, 7, 0xcc)},
	"end-of-range": {
		obj(0, 0, 0, 7, 0xaa),
		{end: moqtransport.EndOfRangeNonExistent, group: 0, object: 5},
		obj(0, 6, 0, 7, 0xbb),
		{end: moqtransport.EndOfRangeUnknown, group: 2, object: 0},
		obj(2, 3, 0, 7, 0xcc),
	},
	"indicator-first": {
		{end: moqtransport.EndOfRangeUnknown, group: 3, object: 1},
		obj(3, 2, 0, 7),
	},
	"indicator-first-group-delta": {
		{end: moqtransport.EndOfRangeUnknown, group: 3, object: 1},
		obj(9, 0, 0, 7),
	},
	"subgroup-id-modes": {
		{group: 1, object: 0, datagram: true, priority: 7},
		obj(2, 0, 4, 9),
		obj(2, 1, 5, 9),
	},
}

type server struct{}

func (server) Handle(ctx context.Context, conn moqtransport.Connection) {
	s := &moqtransport.Session{
		Implementation: "Eyevinn/moqtransport fetchcap",
		FetchHandler: moqtransport.FetchHandlerFunc(func(r *moqtransport.FetchRequest) {
			start, end := r.Range()
			fmt.Fprintf(os.Stderr, "FETCH track=%s start=%v end=%v order=%v\n", r.Track(), start, end, r.GroupOrder())
			script, ok := scripts[r.Track()]
			if !ok {
				_ = r.Reject(moqtransport.RequestErrorDoesNotExist, "no such script")
				return
			}
			response, err := r.Accept()
			if err != nil {
				fmt.Fprintln(os.Stderr, "accept:", err)
				return
			}
			for _, rec := range script {
				if rec.end != 0 {
					err = response.WriteEndOfRange(rec.end, moqtransport.Location{Group: rec.group, Object: rec.object})
				} else {
					o := moqtransport.Object{
						GroupID: rec.group, ObjectID: rec.object, SubgroupID: rec.subgroup,
						Priority: rec.priority, Payload: rec.payload,
					}
					if rec.datagram {
						o.ForwardingPreference = moqtransport.ObjectForwardingPreferenceDatagram
					}
					err = response.WriteObject(o)
				}
				if err != nil {
					fmt.Fprintln(os.Stderr, "write:", err)
					return
				}
			}
			_ = response.Close()
		}),
	}
	if err := s.Run(ctx, conn); err != nil {
		fmt.Fprintln(os.Stderr, "session:", err)
		return
	}
	<-ctx.Done()
}

func client(addr, namespace, track, order string, sg, so, eg, eo uint64) error {
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	q, err := quic.DialAddr(ctx, addr, &tls.Config{
		InsecureSkipVerify: true,
		NextProtos:         []string{moqtransport.SupportedALPNs()[0]},
	}, &quic.Config{EnableDatagrams: true, EnableStreamResetPartialDelivery: true})
	if err != nil {
		return err
	}
	s := &moqtransport.Session{Implementation: "Eyevinn/moqtransport fetchcap"}
	if err := s.Run(ctx, tee.Wrap(quicmoq.NewClient(q), "cli")); err != nil {
		return err
	}
	var opts []moqtransport.FetchOption
	switch order {
	case "descending":
		opts = append(opts, moqtransport.WithFetchGroupOrder(moqtransport.GroupOrderDescending))
	case "ascending":
		opts = append(opts, moqtransport.WithFetchGroupOrder(moqtransport.GroupOrderAscending))
	}
	fs, err := s.Fetch(ctx, splitNamespace(namespace), track,
		moqtransport.Location{Group: sg, Object: so}, moqtransport.Location{Group: eg, Object: eo}, opts...)
	if err != nil {
		return fmt.Errorf("fetch: %w", err)
	}
	fmt.Fprintf(os.Stderr, "FETCH_OK end=%v endOfTrack=%v\n", fs.EndLocation(), fs.EndOfTrack())
	for {
		fo, err := fs.ReadObject(ctx)
		if errors.Is(err, moqtransport.ErrFetchComplete) {
			break
		}
		if err != nil {
			return fmt.Errorf("read: %w", err)
		}
		fmt.Fprintf(os.Stderr, "READ end=%v group=%d object=%d subgroup=%d pref=%v priority=%d payload_len=%d\n",
			fo.EndOfRange, fo.GroupID, fo.ObjectID, fo.SubgroupID, fo.ForwardingPreference, fo.Priority, len(fo.Payload))
	}
	_ = fs.Close()
	time.Sleep(200 * time.Millisecond)
	return nil
}

func splitNamespace(text string) []string {
	var fields []string
	current := ""
	for _, c := range text {
		if c == '/' {
			fields = append(fields, current)
			current = ""
		} else {
			current += string(c)
		}
	}
	return append(fields, current)
}

func u(text string) uint64 {
	v, err := strconv.ParseUint(text, 10, 64)
	if err != nil {
		panic(err)
	}
	return v
}

func main() {
	if len(os.Args) >= 3 && os.Args[1] == "serve" {
		tlsConfig, err := internal.ServerTLSConfig("none.pem", "none.pem")
		if err != nil {
			panic(err)
		}
		if err := internal.RunMoQServer(context.Background(), os.Args[2], tlsConfig, server{}); err != nil {
			panic(err)
		}
		return
	}
	if len(os.Args) == 10 && os.Args[1] == "fetch" {
		a := os.Args
		if err := client(a[2], a[3], a[4], a[5], u(a[6]), u(a[7]), u(a[8]), u(a[9])); err != nil {
			fmt.Fprintln(os.Stderr, "ERROR", err)
			os.Exit(1)
		}
		return
	}
	fmt.Fprintln(os.Stderr, "fetchcap serve <addr> | fetchcap fetch <addr> <namespace> <track> <ascending|descending|none> <startG> <startO> <endG> <endO>")
	os.Exit(2)
}
