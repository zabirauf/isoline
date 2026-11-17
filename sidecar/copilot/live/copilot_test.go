package main

import (
	"math"
	"testing"

	"github.com/github/copilot-sdk/go/rpc"
)

func TestUsdPerMillion(t *testing.T) {
	f := func(v float64) *float64 { return &v }
	n := func(v int64) *int64 { return &v }
	cases := []struct {
		price *float64
		batch *int64
		unit  rpc.TokenPriceUnit
		want  float64
	}{
		{nil, nil, rpc.TokenPriceUnitAiCredits, 0},
		// 10 credits per million tokens is $0.10 per million.
		{f(10), n(1_000_000), rpc.TokenPriceUnitAiCredits, 0.10},
		{f(0.01), n(1000), rpc.TokenPriceUnitAiCredits, 0.10},
		// 10e9 nano-AIU per million tokens is 10 credits.
		{f(10e9), n(1_000_000), rpc.TokenPriceUnitNanoAiu, 0.10},
	}
	for _, c := range cases {
		if got := usdPerMillion(c.price, c.batch, c.unit); math.Abs(got-c.want) > 1e-9 {
			t.Fatalf("usdPerMillion = %v, want %v", got, c.want)
		}
	}
}
