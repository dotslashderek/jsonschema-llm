FROM ruby:3.3-bookworm
WORKDIR /app
COPY bindings/ruby/ ./bindings/ruby/
COPY tests/ ./tests/
WORKDIR /app/bindings/ruby
RUN bundle install
CMD ["bundle", "exec", "rake", "test"]
