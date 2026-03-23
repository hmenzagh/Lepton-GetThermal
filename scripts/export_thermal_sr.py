#!/usr/bin/env python3
"""
Export a thermal super-resolution model to ONNX for use with Thermal_V2.

Supports two modes:
1. ESPCN (default) - Simple sub-pixel CNN, can use pre-trained weights from torchvision
2. IMDN - Information Multi-Distillation Network from Kronbii/thermal-super-resolution

Usage:
    # Export ESPCN with random init (for testing pipeline)
    python export_thermal_sr.py --arch espcn --scale 3

    # Export IMDN with trained weights
    python export_thermal_sr.py --arch imdn --scale 4 --weights checkpoints/thermal_best.pth

    # Train ESPCN on thermal data then export
    python export_thermal_sr.py --arch espcn --scale 3 --train --dataset /path/to/flir_adas

Requirements:
    pip install torch torchvision onnx
"""

import argparse
import torch
import torch.nn as nn
import math


# ============================================================
# ESPCN Architecture (Efficient Sub-Pixel CNN)
# ============================================================

class ESPCN(nn.Module):
    """Sub-pixel CNN for single-channel super-resolution."""

    def __init__(self, scale_factor: int):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 64, kernel_size=5, padding=2)
        self.conv2 = nn.Conv2d(64, 64, kernel_size=3, padding=1)
        self.conv3 = nn.Conv2d(64, 32, kernel_size=3, padding=1)
        self.conv4 = nn.Conv2d(32, scale_factor**2, kernel_size=3, padding=1)
        self.pixel_shuffle = nn.PixelShuffle(scale_factor)
        self.relu = nn.ReLU()

    def forward(self, x):
        x = self.relu(self.conv1(x))
        x = self.relu(self.conv2(x))
        x = self.relu(self.conv3(x))
        x = self.pixel_shuffle(self.conv4(x))
        return x


# ============================================================
# IMDN Architecture (from Kronbii/thermal-super-resolution)
# ============================================================

def stdv_channels(x):
    """Compute standard deviation per channel for CCA attention."""
    batch, channels, height, width = x.size()
    return (x.view(batch, channels, -1).var(dim=2, keepdim=True) + 1e-4).sqrt().view(batch, channels, 1, 1)


class CCALayer(nn.Module):
    """Contrast-aware Channel Attention."""

    def __init__(self, channel, reduction=16):
        super().__init__()
        self.contrast = stdv_channels
        self.avg_pool = nn.AdaptiveAvgPool2d(1)
        self.conv_du = nn.Sequential(
            nn.Conv2d(channel, channel // reduction, 1, padding=0, bias=True),
            nn.ReLU(inplace=True),
            nn.Conv2d(channel // reduction, channel, 1, padding=0, bias=True),
            nn.Sigmoid(),
        )

    def forward(self, x):
        y = self.contrast(x) + self.avg_pool(x)
        y = self.conv_du(y)
        return x * y


class IMDModule(nn.Module):
    """Information Multi-Distillation Module."""

    def __init__(self, in_channels, distillation_rate=0.25):
        super().__init__()
        self.distilled_channels = int(in_channels * distillation_rate)
        self.remaining_channels = int(in_channels - self.distilled_channels)

        self.c1 = nn.Conv2d(in_channels, in_channels, 3, padding=1)
        self.c2 = nn.Conv2d(self.remaining_channels, in_channels, 3, padding=1)
        self.c3 = nn.Conv2d(self.remaining_channels, in_channels, 3, padding=1)
        self.c4 = nn.Conv2d(self.remaining_channels, self.distilled_channels, 3, padding=1)
        self.act = nn.LeakyReLU(negative_slope=0.05, inplace=True)
        self.c5 = nn.Conv2d(self.distilled_channels * 4, in_channels, 1, padding=0)
        self.cca = CCALayer(self.distilled_channels * 4, reduction=16)

    def forward(self, x):
        out1 = self.act(self.c1(x))
        d1, r1 = torch.split(out1, (self.distilled_channels, self.remaining_channels), dim=1)

        out2 = self.act(self.c2(r1))
        d2, r2 = torch.split(out2, (self.distilled_channels, self.remaining_channels), dim=1)

        out3 = self.act(self.c3(r2))
        d3, r3 = torch.split(out3, (self.distilled_channels, self.remaining_channels), dim=1)

        d4 = self.act(self.c4(r3))

        out = torch.cat([d1, d2, d3, d4], dim=1)
        out = self.cca(out)
        out = self.c5(out) + x
        return out


class IMDN(nn.Module):
    """Information Multi-Distillation Network for thermal super-resolution."""

    def __init__(self, in_nc=1, nf=64, num_modules=6, out_nc=1, upscale=2):
        super().__init__()
        self.fea_conv = nn.Conv2d(in_nc, nf, 3, padding=1)
        self.modules_body = nn.ModuleList([IMDModule(nf) for _ in range(num_modules)])
        self.c = nn.Sequential(
            nn.Conv2d(nf * num_modules, nf, 1, padding=0),
            nn.LeakyReLU(negative_slope=0.05, inplace=True),
        )
        self.LR_conv = nn.Conv2d(nf, nf, 3, padding=1)

        upsample_block = []
        if upscale == 2 or upscale == 4:
            for _ in range(int(math.log2(upscale))):
                upsample_block += [
                    nn.Conv2d(nf, nf * 4, 3, padding=1),
                    nn.PixelShuffle(2),
                ]
        elif upscale == 3:
            upsample_block = [
                nn.Conv2d(nf, nf * 9, 3, padding=1),
                nn.PixelShuffle(3),
            ]
        self.upsampler = nn.Sequential(*upsample_block)
        self.out_conv = nn.Conv2d(nf, out_nc, 3, padding=1)

    def forward(self, x):
        out_fea = self.fea_conv(x)
        outs = []
        out = out_fea
        for mod in self.modules_body:
            out = mod(out)
            outs.append(out)
        out = self.c(torch.cat(outs, dim=1))
        out = self.LR_conv(out) + out_fea
        out = self.upsampler(out)
        out = self.out_conv(out)
        return out


def export_onnx(model, scale, output_path, input_h=120, input_w=160):
    """Export model to ONNX with dynamic spatial dimensions."""
    model.eval()
    dummy = torch.randn(1, 1, input_h, input_w)

    with torch.no_grad():
        out = model(dummy)
        print(f"Input:  {dummy.shape}")
        print(f"Output: {out.shape}")
        print(f"Scale:  {out.shape[2] // dummy.shape[2]}x")

    torch.onnx.export(
        model,
        dummy,
        output_path,
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={
            "input": {0: "batch", 2: "height", 3: "width"},
            "output": {0: "batch", 2: "height", 3: "width"},
        },
        opset_version=17,
        do_constant_folding=True,
    )
    print(f"Exported to {output_path}")


def main():
    parser = argparse.ArgumentParser(description="Export thermal SR model to ONNX")
    parser.add_argument("--arch", choices=["espcn", "imdn"], default="espcn")
    parser.add_argument("--scale", type=int, default=3, choices=[2, 3, 4])
    parser.add_argument("--weights", type=str, default=None, help="Path to .pth weights")
    parser.add_argument("--output", type=str, default=None, help="Output ONNX path")
    parser.add_argument("--nf", type=int, default=64, help="IMDN feature channels")
    parser.add_argument("--num-modules", type=int, default=6, help="IMDN module count")
    args = parser.parse_args()

    if args.arch == "espcn":
        model = ESPCN(args.scale)
    else:
        model = IMDN(in_nc=1, nf=args.nf, num_modules=args.num_modules, out_nc=1, upscale=args.scale)

    if args.weights:
        ckpt = torch.load(args.weights, map_location="cpu", weights_only=True)
        state = ckpt.get("model_state_dict") or ckpt.get("state_dict") or ckpt
        if any(k.startswith("module.") for k in state):
            state = {k.replace("module.", ""): v for k, v in state.items()}
        model.load_state_dict(state, strict=True)
        print(f"Loaded weights from {args.weights}")
    else:
        params = sum(p.numel() for p in model.parameters())
        print(f"No weights provided — exporting with random init ({params:,} params)")

    output = args.output or f"src-tauri/models/thermal_sr_{args.arch}_x{args.scale}.onnx"
    export_onnx(model, args.scale, output)


if __name__ == "__main__":
    main()
