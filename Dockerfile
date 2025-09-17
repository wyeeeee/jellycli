# Use a slim base image with a specific version for stability
FROM debian:bullseye-slim

# Set working directory
WORKDIR /app

# Copy the pre-compiled binary from the build context
# The CI/CD pipeline will place the binary here.
COPY jellycli .

# Expose the port the app runs on
EXPOSE 7878

# Create directories for volumes and set permissions for a non-root user
RUN mkdir -p /app/credentials /app/logs && chown -R 1001:1001 /app

# Define volumes for config, credentials, and logs
VOLUME ["/app/credentials", "/app/logs"]

# Switch to a non-root user for security
USER 1001

# Set the entrypoint to run the application
CMD ["./jellycli"]