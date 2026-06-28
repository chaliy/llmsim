// Anthropic client example for llmsim (Go).
//
// Demonstrates connecting to a running llmsim server using the official
// Anthropic Go SDK. The server simulates Claude responses with realistic
// latency without running actual models.
//
// Server endpoints:
//
//	POST /anthropic/v1/messages    - Messages API (streaming supported)
//	GET  /anthropic/v1/models      - List available Claude models
//
// Prerequisites:
//
//	Start the llmsim server first:
//	    llmsim serve --port 8080
//
// The module (go.mod / go.sum) is committed alongside this file, so no manual
// setup is needed — just run it.
//
// Usage:
//
//	cd examples/go
//	LLMSIM_URL=http://localhost:8080/anthropic/ go run anthropic_client.go
//
// Note: the base URL must end with a trailing slash and include the
// /anthropic/ prefix; the SDK appends "v1/messages" to it.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/anthropics/anthropic-sdk-go"
	"github.com/anthropics/anthropic-sdk-go/option"
)

func main() {
	baseURL := os.Getenv("LLMSIM_URL")
	if baseURL == "" {
		baseURL = "http://localhost:8080/anthropic/"
	}

	fmt.Println("==================================================")
	fmt.Println("Anthropic SDK (Go) + LLMSim Example")
	fmt.Printf("Connecting to: %s\n", baseURL)
	fmt.Println("==================================================")

	// The simulator ignores the API key, but the SDK requires one.
	client := anthropic.NewClient(
		option.WithBaseURL(baseURL),
		option.WithAPIKey("not-needed"),
	)
	ctx := context.Background()

	// Example 1: Simple message
	fmt.Println("\n1. Simple Message")
	fmt.Println("------------------------------")
	msg, err := client.Messages.New(ctx, anthropic.MessageNewParams{
		Model:     "claude-opus-4-8",
		MaxTokens: 128,
		System: []anthropic.TextBlockParam{
			{Text: "You are a helpful assistant."},
		},
		Messages: []anthropic.MessageParam{
			anthropic.NewUserMessage(anthropic.NewTextBlock("What is the capital of France?")),
		},
	})
	if err != nil {
		fmt.Printf("Error: %v\n", err)
		fmt.Println("\nMake sure the llmsim server is running:")
		fmt.Println("  llmsim serve --port 8080")
		os.Exit(1)
	}
	for _, block := range msg.Content {
		if block.Type == "text" {
			fmt.Printf("Response: %s\n", block.Text)
		}
	}
	fmt.Printf("Model: %s | stop_reason: %s\n", msg.Model, msg.StopReason)
	fmt.Printf("Tokens: in=%d out=%d\n", msg.Usage.InputTokens, msg.Usage.OutputTokens)

	// Example 2: Streaming
	fmt.Println("\n2. Streaming Response")
	fmt.Println("------------------------------")
	fmt.Print("Response: ")
	stream := client.Messages.NewStreaming(ctx, anthropic.MessageNewParams{
		Model:     "claude-haiku-4-5",
		MaxTokens: 128,
		Messages: []anthropic.MessageParam{
			anthropic.NewUserMessage(anthropic.NewTextBlock("Tell me a short story.")),
		},
	})
	acc := anthropic.Message{}
	for stream.Next() {
		event := stream.Current()
		_ = acc.Accumulate(event)
		switch d := event.AsAny().(type) {
		case anthropic.ContentBlockDeltaEvent:
			if td, ok := d.Delta.AsAny().(anthropic.TextDelta); ok {
				fmt.Print(td.Text)
			}
		}
	}
	if err := stream.Err(); err != nil {
		fmt.Printf("\nstream error: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("\n(streamed, output tokens: %d)\n", acc.Usage.OutputTokens)

	fmt.Println("\n==================================================")
	fmt.Println("Examples complete!")
	fmt.Println("==================================================")
}
