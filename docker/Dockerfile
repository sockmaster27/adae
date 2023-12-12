# Create an image containing the dependencies required for the CI pipeline

FROM ubuntu:latest

# Update apt-get index
RUN apt-get update

# Install utilities
RUN apt-get install -y curl
RUN apt-get install -y pkg-config
RUN apt-get install -y build-essential

# Install ALSA
RUN apt-get install -y libasound2-dev

# Install Rust toolchain
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y
ENV PATH="/root/.cargo/bin:$PATH"
RUN rustup default stable
