package main

import (
	"bufio"
	"encoding/hex"
	"flag"
	"fmt"
	"net"
	"os"
	"strings"

	"google.golang.org/grpc"

	"sidecar-go/internal/server"
	"sidecar-go/proto/agentpb"
)

func main() {
	port := flag.Int("port", 50051, "port to listen on")
	shutdownOnStdin := flag.Bool("shutdown-on-stdin", false, "shutdown when SHUTDOWN received on stdin")
	mindRoot := flag.String("mind-root", "", "mind root directory")
	flag.Parse()

	if *mindRoot == "" {
		fmt.Fprintln(os.Stderr, "Error: --mind-root is required")
		os.Exit(1)
	}

	reader := bufio.NewReader(os.Stdin)
	authLine, err := reader.ReadString('\n')
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error reading auth from stdin: %v\n", err)
		os.Exit(1)
	}

	authLine = strings.TrimSpace(authLine)
	if !strings.HasPrefix(authLine, "AUTH ") {
		fmt.Fprintln(os.Stderr, "Error: expected AUTH token on stdin")
		os.Exit(1)
	}

	token := strings.TrimPrefix(authLine, "AUTH ")
	tokenBytes, err := hex.DecodeString(token)
	if err != nil || len(tokenBytes) != 32 {
		fmt.Fprintln(os.Stderr, "Error: invalid authentication token")
		os.Exit(1)
	}

	lis, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", *port))
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error binding to port %d: %v\n", *port, err)
		os.Exit(1)
	}

	boundPort := lis.Addr().(*net.TCPAddr).Port

	grpcServer := grpc.NewServer()
	agentpb.RegisterAgentRuntimeServer(grpcServer, server.NewRuntimeServer(token, *mindRoot))

	fmt.Printf("READY %d\n", boundPort)
	_ = os.Stdout.Sync()

	if *shutdownOnStdin {
		serveErr := make(chan error, 1)
		go func() {
			serveErr <- grpcServer.Serve(lis)
		}()

		// Wait for shutdown signal on stdin
		cmd, _ := reader.ReadString('\n')
		cmd = strings.TrimSpace(cmd)
		if cmd == "SHUTDOWN" || cmd == "" {
			grpcServer.GracefulStop()
		} else {
			grpcServer.Stop()
			fmt.Fprintf(os.Stderr, "Unexpected stdin command: %s\n", cmd)
			os.Exit(1)
		}
	} else {
		if err := grpcServer.Serve(lis); err != nil {
			fmt.Fprintf(os.Stderr, "gRPC server error: %v\n", err)
			os.Exit(1)
		}
	}
}
